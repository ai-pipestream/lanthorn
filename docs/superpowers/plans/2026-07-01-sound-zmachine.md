# Sound Support — Cross-platform Audio Backend + Z-Machine Sound Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add real audio to babelmap — synthesised tones for the built-in Z-machine bleeps (#1/#2) and sampled playback (AIFF/Ogg/MOD) of Blorb `Snd ` resources (#≥3), in the `app` TUI and `zvm-cli`, with a finish-routine callback — while keeping the `zvm`/`gvm` VM crates zero-dependency.

**Architecture:** A new host-side `crates/audio` crate wraps `rodio` (feature-gated, degrades to a compile-time / runtime no-op). `crates/blorb` gains a `sound(number)` accessor. `crates/zvm` records a richer `SoundEvent` (replacing the old `Beep`/`pending_beeps`) and exposes a general `run_routine`. The hosts own an `AudioBackend` + the loaded Blorb, drain sound events each turn, play tones/samples, and poll for completion to fire finish routines.

**Tech Stack:** Rust (workspace, edition 2021), `rodio` 0.19 (CoreAudio/WASAPI/ALSA), `mod_player` 0.1 (pure-`std` ProTracker MOD decoder), `toml_edit`/`serde` (app config), `crossterm` (CLI/app terminal).

## Global Constraints

- VM crates `zvm` and `gvm` stay ZERO external dependencies — all audio lives in the hosts + `crates/audio`.
- Must build+run on macOS, Windows, Linux. `crates/audio` `playback` feature (default on) can be disabled for headless/CI; then the backend is a compile-time no-op. `mod-music` (default on) gates `mod_player`.
- Runtime: no output device → degrade to silent (log once, never panic/block). Non-blocking throughout.
- Config `volume` is `u8` 0..=100 (master); Z-event volume is 1..8 (255=loudest); combine as gain.
- On Linux the `playback` feature needs the ALSA dev headers (`libasound2-dev` / `alsa-lib-devel`).
- Commit trailers on EVERY commit (exact):
  ```
  Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
- Validation target: The Lurking Horror (`stories/lurkinghorror-r219-s870912.z3` + sibling `stories/Lurking.blb`) plays sampled sounds in both hosts; `--no-sound`/`toggle-sound` silences; `--volume`/`/volume` scales; border pulse still fires.

---

## File Structure

| File | Responsibility |
|------|----------------|
| `crates/blorb/src/lib.rs` (modify) | `SoundKind` enum + `Blorb::sound(number)` accessor returning payload bytes + detected format. |
| `crates/zvm/src/cpu/exec.rs` (modify) | `SoundEvent` struct + `pending_sounds` field (replace `Beep`/`pending_beeps`); `sound_effect` records events; extract `run_routine` from `run_timed_interrupt`. Zero-dep. |
| `Cargo.toml` (modify) | Add `crates/audio` to workspace members. |
| `crates/audio/Cargo.toml` (create) | New crate: feature gates (`playback`, `mod-music`), `rodio` + `mod_player` optional deps. |
| `crates/audio/src/lib.rs` (create) | `AudioBackend`, `SoundFormat`, `SoundId`; tone synth, gain, AIFF decoder, Ogg-via-rodio, MOD via `ModSource`. Real + no-op impls behind `#[cfg]`. |
| `crates/app/src/session.rs` (modify) | `TurnResult.sounds: Vec<SoundEvent>` (replace `beep`); `run_sound_finish`. |
| `crates/app/src/state.rs` (modify) | `BeepKind` enum; `SoundPulse.kind: BeepKind`; `AppState` audio/blorb/sound-map fields; `play_turn_sounds`. |
| `crates/app/src/config.rs` (modify) | `enable_sound: bool` + `volume: u8` config surface. |
| `crates/app/src/slash.rs` (modify) | `toggle-sound` + `volume <0-100>` commands. |
| `crates/app/src/input.rs` (modify) | `Action::ToggleSound` / `Action::SetVolume`; config-screen toggle/cycle rows. |
| `crates/app/src/render/config_screen.rs` (modify) | `enable_sound` + `volume` settings rows. |
| `crates/app/src/main.rs` (modify) | Border pulse reads `BeepKind`; resolve Blorb + construct backend; play sounds in `apply_turn_events`; poll completions → `run_sound_finish`. |
| `crates/app/Cargo.toml` (modify) | Add `audio = { path = "../audio" }`. |
| `crates/zvm-cli/src/main.rs` (modify) | `--no-sound` / `--volume`; own `AudioBackend` + Blorb; drain events → play; poll completions → `run_routine`. |
| `crates/zvm-cli/Cargo.toml` (modify) | Add `audio = { path = "../audio" }`. |
| `README.md` (modify) | Audio feature docs + Linux ALSA prereq + flags/commands. |

---

## Task 1: `crates/blorb` — `sound()` accessor

**Files:**
- Modify: `crates/blorb/src/lib.rs` (add `SoundKind` after `ExecKind` at :41; add `sound()` after `resource()` at :149; add a test after :291)

**Interfaces:**
- Produces:
  - `pub enum SoundKind { Aiff, Ogg, Mod, Other }` (derives `Debug, Clone, Copy, PartialEq, Eq`)
  - `pub fn Blorb::sound(&self, number: u32) -> Option<(&[u8], SoundKind)>`

- [ ] **Step 1: Write the failing test**

Append inside the existing `mod tests` block (after the `resource_handles_odd_length_pad_byte` test that ends at `crates/blorb/src/lib.rs:291`):

```rust
    #[test]
    fn sound_fetches_aiff_by_number() {
        let b = build_blorb(&[
            (b"Exec", 0, b"ZCOD", b"abcd"),
            (b"Snd ", 7, b"FORM", b"aiffbytes"),
        ]);
        let blorb = Blorb::parse(b).unwrap();
        let (data, kind) = blorb.sound(7).unwrap();
        assert_eq!(data, b"aiffbytes");
        assert_eq!(kind, SoundKind::Aiff);
        assert!(blorb.sound(99).is_none(), "absent sound number returns None");
    }

    #[test]
    fn sound_detects_ogg_mod_other() {
        let b = build_blorb(&[
            (b"Snd ", 1, b"OGGV", b"ogg"),
            (b"Snd ", 2, b"MOD ", b"mod"),
            (b"Snd ", 3, b"AIFF", b"weird"),
        ]);
        let blorb = Blorb::parse(b).unwrap();
        assert_eq!(blorb.sound(1).unwrap().1, SoundKind::Ogg);
        assert_eq!(blorb.sound(2).unwrap().1, SoundKind::Mod);
        assert_eq!(blorb.sound(3).unwrap().1, SoundKind::Other);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p blorb sound_fetches_aiff_by_number`
Expected: FAIL — `no method named sound found for struct Blorb` / `cannot find type SoundKind`.

- [ ] **Step 3: Write minimal implementation**

Add the enum right after the `ExecKind` enum (after `crates/blorb/src/lib.rs:41`):

```rust
/// The kind of a Blorb `Snd ` sound resource, detected from its chunk type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundKind {
    /// AIFF sampled sound (`FORM` chunk).
    Aiff,
    /// Ogg Vorbis sampled sound (`OGGV` chunk).
    Ogg,
    /// Amiga ProTracker module (`MOD ` chunk).
    Mod,
    /// A sound resource whose chunk type we do not decode.
    Other,
}
```

Add the accessor inside `impl Blorb`, right after the `resource` method (after `crates/blorb/src/lib.rs:149`, i.e. after its closing `}` and before the `impl`'s closing brace at :150):

```rust
    /// Payload bytes + detected [`SoundKind`] for sound resource `number`
    /// (`usage == b"Snd "`), or `None` when no such resource exists. The kind is
    /// detected from the chunk type: `FORM` → AIFF, `OGGV` → Ogg, `MOD ` → Mod,
    /// anything else → Other.
    pub fn sound(&self, number: u32) -> Option<(&[u8], SoundKind)> {
        let e = self
            .index
            .iter()
            .find(|r| &r.usage == b"Snd " && r.number == number)?;
        let kind = match &e.chunk_type {
            b"FORM" => SoundKind::Aiff,
            b"OGGV" => SoundKind::Ogg,
            b"MOD " => SoundKind::Mod,
            _ => SoundKind::Other,
        };
        Some((self.chunk_data(e), kind))
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p blorb sound_`
Expected: PASS (`sound_fetches_aiff_by_number`, `sound_detects_ogg_mod_other`).

- [ ] **Step 5: Commit**

```bash
git add crates/blorb/src/lib.rs
git commit -m "feat(blorb): add sound() accessor + SoundKind

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Task 2: `crates/zvm` — SoundEvent model + host beep migration

This task spans `zvm` + `app` + `zvm-cli` because removing `Beep`/`pending_beeps` breaks both hosts. **No audio is produced** — the border pulse and terminal bell behave exactly as before. It is one cohesive, independently-testable task.

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` (replace `Beep` enum at :27-31; replace `pending_beeps` field at :122-123 and its init at :184; rewrite `sound_effect` handler at :1136-1150; migrate tests at :4671-4699)
- Modify: `crates/app/src/session.rs` (imports :16; `TurnResult.beep` at :134; `drain_turn` at :334-336, :346; construction sites :831, :850, :877, :903, :923; test :1361, :1364)
- Modify: `crates/app/src/state.rs` (`BeepKind` enum + `SoundPulse.kind` at :382-385; tests :1886-1899)
- Modify: `crates/app/src/main.rs` (border render :408-419; `apply_turn_events` :4273-4275; construction sites :1286, :2646, :2786, :2907, :2975, :3248, :3442, :4216)
- Modify: `crates/zvm-cli/src/main.rs` (bell drain :663-668)

**Interfaces:**
- Produces (zvm):
  - `pub struct SoundEvent { pub number: u16, pub effect: u8, pub volume: u8, pub repeats: u8, pub routine: u16 }` (derives `Debug, Clone, Copy, PartialEq, Eq`)
  - `pub pending_sounds: Vec<SoundEvent>` field on `Machine`
  - `Beep` and `pending_beeps` are REMOVED.
- Produces (app):
  - `pub enum BeepKind { High, Low }` (derives `Debug, Clone, Copy`) in `crate::state`
  - `TurnResult.sounds: Vec<zvm::cpu::exec::SoundEvent>` (replaces `beep`)
  - `SoundPulse.kind: crate::state::BeepKind` (replaces `zvm::cpu::exec::Beep`)

- [ ] **Step 1: Write the failing engine tests**

Replace the three tests at `crates/zvm/src/cpu/exec.rs:4671-4699` (`sound_effect_records_high_and_low_beeps`, `sound_effect_sampled_sound_records_diagnostic_no_beep`, `sound_effect_zero_records_nothing`) with:

```rust
    #[test]
    fn sound_effect_records_high_and_low_bleeps() {
        let mut m = build_test_machine(&[]);
        m.exec_var(0x15, &[1], None, None);
        m.exec_var(0x15, &[2], None, None);
        assert_eq!(m.pending_sounds.len(), 2);
        // No volume operand -> vw defaults to 8: volume 8, repeats 0.
        assert_eq!(m.pending_sounds[0], SoundEvent { number: 1, effect: 0, volume: 8, repeats: 0, routine: 0 });
        assert_eq!(m.pending_sounds[1], SoundEvent { number: 2, effect: 0, volume: 8, repeats: 0, routine: 0 });
        assert!(m.diagnostics.is_empty(), "bleeps must not record diagnostics");
    }

    #[test]
    fn sound_effect_records_sampled_sound_event_no_diagnostic() {
        let mut m = build_test_machine(&[]);
        // number 5, effect 2 (start), volume word 0xFF03 -> volume 3, repeats 255 (forever), routine 0x1234
        m.exec_var(0x15, &[5, 2, 0xFF03, 0x1234], None, None);
        assert_eq!(
            m.pending_sounds,
            vec![SoundEvent { number: 5, effect: 2, volume: 3, repeats: 255, routine: 0x1234 }]
        );
        assert!(m.diagnostics.is_empty(), "sampled sounds are recorded, not dropped as diagnostics");
    }

    #[test]
    fn sound_effect_zero_records_nothing() {
        let mut m = build_test_machine(&[]);
        m.exec_var(0x15, &[0], None, None);
        assert!(m.pending_sounds.is_empty());
        assert!(m.diagnostics.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zvm sound_effect`
Expected: FAIL — `no field pending_sounds` / `cannot find struct SoundEvent`.

- [ ] **Step 3: Implement the SoundEvent model in the engine**

In `crates/zvm/src/cpu/exec.rs`, replace the `Beep` enum (`:25-31`):

```rust
/// A built-in Z-machine bleep recorded by `sound_effect` (ZMSD §9.4):
/// sound #1 is the high-pitched bleep, #2 the low-pitched bleep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beep {
    High,
    Low,
}
```

with:

```rust
/// A Z-machine `sound_effect` event (ZMSD §9.4), recorded for the host to act on.
/// `number` 1/2 are the built-in high/low bleeps; `number >= 3` selects a Blorb
/// `Snd ` resource. `effect`: 1=prepare 2=start 3=stop 4=finish. `volume` is the
/// Z-scale 1..=8 (255 = loudest). `repeats` is the repeat count (0/255 = forever).
/// `routine` (v5+) is the finish-routine the host calls when the sound ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundEvent {
    pub number: u16,
    pub effect: u8,
    pub volume: u8,
    pub repeats: u8,
    pub routine: u16,
}
```

Replace the field at `:122-123`:

```rust
    /// Bleeps recorded by `sound_effect` since the host last drained them.
    pub pending_beeps: Vec<Beep>,
```

with:

```rust
    /// Sound events recorded by `sound_effect` since the host last drained them.
    pub pending_sounds: Vec<SoundEvent>,
```

Replace the init at `:184` (`pending_beeps: Vec::new(),`) with:

```rust
            pending_sounds: Vec::new(),
```

Rewrite the `sound_effect` handler at `:1132-1151` (the `// 0x15 sound_effect` comment through the arm's closing `}`):

```rust
            // 0x15 sound_effect — number effect volume routine (ZMSD §9.4).
            // Record a SoundEvent for every call (including #1/#2 bleeps). The host
            // drains `pending_sounds` and decides what to play / how to visualise.
            0x15 => {
                let number = ops.first().copied().unwrap_or(0);
                if number != 0 {
                    let effect = ops.get(1).copied().unwrap_or(0) as u8;
                    // Volume word: low byte = volume (1..8, 255=loudest), high byte
                    // = repeat count (0/255 = forever). Default 8 when omitted.
                    let vw = ops.get(2).copied().unwrap_or(8);
                    let volume = (vw & 0xFF) as u8;
                    let repeats = (vw >> 8) as u8;
                    let routine = ops.get(3).copied().unwrap_or(0);
                    self.pending_sounds.push(SoundEvent { number, effect, volume, repeats, routine });
                }
                StepResult::Continue
            }
```

- [ ] **Step 4: Run engine tests to verify they pass**

Run: `cargo test -p zvm sound_effect`
Expected: PASS (3 tests).

- [ ] **Step 5: Migrate the app (`session.rs`, `state.rs`, `main.rs`) and `zvm-cli`**

**`crates/app/src/session.rs`** — imports at `:16`, replace:

```rust
use zvm::cpu::exec::{Beep, Machine, StepResult};
```

with:

```rust
use zvm::cpu::exec::{Machine, SoundEvent, StepResult};
```

Replace the `TurnResult` field at `:133-134`:

```rust
    /// The latest bleep emitted this turn (last wins), if any.
    pub beep: Option<Beep>,
```

with:

```rust
    /// Sound events emitted this turn (drained from the VM), in order.
    pub sounds: Vec<SoundEvent>,
```

In `drain_turn`, replace `:335-336`:

```rust
        let beep = self.machine.pending_beeps.last().copied();
        self.machine.pending_beeps.clear();
```

with:

```rust
        let sounds = std::mem::take(&mut self.machine.pending_sounds);
```

and in the returned struct literal replace `:346` (`beep,`) with:

```rust
            sounds,
```

Update the five test construction sites (`crates/app/src/session.rs:831, :850, :877, :903, :923`): replace each `beep: None,` with `sounds: Vec::new(),`.

Update the test at `:1361` and `:1364`:

```rust
        assert!(r.beep.is_none(), "no beep when the game emits no sound");
```
→
```rust
        assert!(r.sounds.is_empty(), "no sounds when the game emits no sound");
```
and
```rust
        assert!(sess.machine.pending_beeps.is_empty());
```
→
```rust
        assert!(sess.machine.pending_sounds.is_empty());
```

**`crates/app/src/state.rs`** — replace `SoundPulse` at `:380-385`:

```rust
/// An in-flight one-shot story-border flash triggered by a `sound_effect` bleep.
#[derive(Debug)]
pub struct SoundPulse {
    pub kind: zvm::cpu::exec::Beep,
    pub started: std::time::Instant,
}
```

with:

```rust
/// A host-side bleep classification for the border-pulse visual cue.
#[derive(Debug, Clone, Copy)]
pub enum BeepKind {
    High,
    Low,
}

/// An in-flight one-shot story-border flash triggered by a `sound_effect` bleep.
#[derive(Debug)]
pub struct SoundPulse {
    pub kind: BeepKind,
    pub started: std::time::Instant,
}
```

Update the two tests: replace `use zvm::cpu::exec::Beep;` at `:1886` and `:1895` with `use crate::state::BeepKind;`, and every `Beep::High` at `:1889`, `:1890`, `:1899` with `BeepKind::High`.

**`crates/app/src/main.rs`** — border render at `:408-419`, replace:

```rust
                    let beep_color = match p.kind {
                        zvm::cpu::exec::Beep::High => state
                            .colors
                            .sound_beep_high
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(255, 180, 40)),
                        zvm::cpu::exec::Beep::Low => state
                            .colors
                            .sound_beep_low
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(60, 140, 220)),
                    };
```

with:

```rust
                    let beep_color = match p.kind {
                        app::state::BeepKind::High => state
                            .colors
                            .sound_beep_high
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(255, 180, 40)),
                        app::state::BeepKind::Low => state
                            .colors
                            .sound_beep_low
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(60, 140, 220)),
                    };
```

In `apply_turn_events`, replace `:4273-4275`:

```rust
    if let Some(kind) = result.beep {
        state.sound_pulse = Some(SoundPulse { kind, started: std::time::Instant::now() });
    }
```

with (last bleep — number 1 or 2 — wins, preserving today's last-wins behaviour):

```rust
    if let Some(kind) = result.sounds.iter().rev().find_map(|ev| match ev.number {
        1 => Some(app::state::BeepKind::High),
        2 => Some(app::state::BeepKind::Low),
        _ => None,
    }) {
        state.sound_pulse = Some(SoundPulse { kind, started: std::time::Instant::now() });
    }
```

Update the eight `TurnResult` construction sites in `main.rs` (`:1286, :2646, :2786, :2907, :2975, :3248, :3442, :4216`): replace each `beep: None,` with `sounds: Vec::new(),`.

**`crates/zvm-cli/src/main.rs`** — replace the bell drain at `:662-668`:

```rust
        // Bleeps: drain and ring (TTY only).
        let beeps = machine.pending_beeps.len();
        machine.pending_beeps.clear();
        if beeps > 0 {
            print!("{}", screen::bleep_bytes(beeps, stdout_is_tty));
            let _ = io::stdout().flush();
        }
```

with (count bleep events; audio comes in Task 10):

```rust
        // Bleeps: drain sound events and ring for #1/#2 (TTY only). Sampled audio
        // playback is wired in a later task; here we only preserve the terminal bell.
        let beeps = machine
            .pending_sounds
            .drain(..)
            .filter(|e| e.number == 1 || e.number == 2)
            .count();
        if beeps > 0 {
            print!("{}", screen::bleep_bytes(beeps, stdout_is_tty));
            let _ = io::stdout().flush();
        }
```

- [ ] **Step 6: Run the workspace build + tests to verify green**

Run: `cargo build --workspace`
Expected: builds cleanly (no `Beep` / `pending_beeps` references remain).

Run: `cargo test -p zvm -p app && cargo test -p zvm-cli`
Expected: PASS (engine sound tests, app session/state tests, cli tests).

- [ ] **Step 7: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs crates/app/src/session.rs crates/app/src/state.rs crates/app/src/main.rs crates/zvm-cli/src/main.rs
git commit -m "refactor(zvm): SoundEvent model replaces Beep; migrate hosts

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Task 3: `crates/zvm` — extract `run_routine`

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` (extract `run_routine` from `run_timed_interrupt` at :1520-1562; add tests near the existing `run_timed_interrupt` tests at :3825-3873)

**Interfaces:**
- Produces: `pub fn Machine::run_routine(&mut self, packed_routine: u16) -> u16` — calls the packed routine to completion and returns its value; does not disturb `pending_input` on the normal path and restores it on a nested-input bail.
- Consumes: `call_routine`, `self.step()`, `StepResult` (existing).

- [ ] **Step 1: Write the failing tests**

Add after the `run_timed_interrupt_continue_and_side_effect` test (`crates/zvm/src/cpu/exec.rs:3873`), using the existing `timed_read_story` helper (`:3800`):

```rust
    #[test]
    fn run_routine_returns_true_value() {
        // routine body: rtrue (0OP 0xB0) -> returns 1.
        let (buf, rout) = timed_read_story(&[0xB0]);
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        let packed = (rout / 4) as u16;
        assert_eq!(m.run_routine(packed), 1, "rtrue routine returns 1");
        assert!(m.pending_input.is_none(), "no read pending -> pending_input stays None");
    }

    #[test]
    fn run_routine_returns_explicit_value() {
        // routine body: ret 7 (1OP:0x0B short form small constant 7): 0x9B 0x07.
        let (buf, rout) = timed_read_story(&[0x9B, 0x07]);
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        let packed = (rout / 4) as u16;
        assert_eq!(m.run_routine(packed), 7, "ret 7 returns 7");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zvm run_routine`
Expected: FAIL — `no method named run_routine found`.

- [ ] **Step 3: Extract `run_routine` and rewrite `run_timed_interrupt`**

In `crates/zvm/src/cpu/exec.rs`, replace the entire body of `run_timed_interrupt` at `:1520-1562` (from `pub fn run_timed_interrupt` through its final `}` at :1562) with:

```rust
    pub fn run_timed_interrupt(&mut self) -> TimedInterrupt {
        let saved = match self.pending_input {
            Some(p) if p.interrupt_routine != 0 => p, // PendingInput: Copy
            _ => return TimedInterrupt { aborted: false },
        };
        let ret = self.run_routine(saved.interrupt_routine);
        TimedInterrupt { aborted: ret != 0 }
    }

    /// Call `packed_routine` to completion and return its value. Safe whether or
    /// not a read is pending: it snapshots `pending_input` and restores it if the
    /// routine attempts nested input/save/restart (unsupported — the routine is
    /// then abandoned and 0 is returned). On the normal path `pending_input` is
    /// left untouched. Used by timed-input interrupts and by the sound
    /// finish-routine callback.
    pub fn run_routine(&mut self, packed_routine: u16) -> u16 {
        let saved = self.pending_input; // Option<PendingInput>: Copy
        let base_frames = self.state.frames.len();
        let base_stack = self.state.eval_stack.len();
        // Push the routine, storing its return value onto the eval stack (var 0).
        call_routine(&mut self.state, &mut self.mem, packed_routine, &[], Some(0));
        if self.state.frames.len() == base_frames {
            // packed 0 / bad addr: call_routine pushed 0 to the stack already.
            return self.state.eval_stack.pop().unwrap_or(0);
        }
        loop {
            match self.step() {
                StepResult::Continue => {
                    if self.state.frames.len() <= base_frames {
                        break;
                    }
                }
                // Nested input/save/restart/quit inside the routine: unsupported.
                // Unwind and restore, including pending_input (a nested read opcode
                // may have overwritten it).
                _ => {
                    self.state.frames.truncate(base_frames);
                    self.state.eval_stack.truncate(base_stack);
                    self.pending_input = saved;
                    return 0;
                }
            }
        }
        let ret = self.state.eval_stack.pop().unwrap_or(0);
        // Guard: a well-behaved routine leaves the stack where we started.
        self.state.eval_stack.truncate(base_stack);
        ret
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p zvm run_routine run_timed_interrupt`
Expected: PASS — the two new `run_routine` tests AND the existing `run_timed_interrupt_abort_when_routine_true` / `run_timed_interrupt_continue_and_side_effect` (now delegating through `run_routine`).

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs
git commit -m "refactor(zvm): extract run_routine from run_timed_interrupt

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Task 4: `crates/audio` — crate scaffold + tones (Audio Task A)

**Files:**
- Modify: `Cargo.toml` (add member)
- Create: `crates/audio/Cargo.toml`
- Create: `crates/audio/src/lib.rs`

> **Version note:** verify the resolved `rodio` version after `cargo build` (`cargo tree -p rodio`). The code below targets rodio 0.19 (`OutputStream::try_default()`, `Sink::try_new(&handle)`, `SamplesBuffer::new(channels, sample_rate, samples)`, `sink.empty()`, `sink.set_volume()`, `sink.stop()`, `Source::repeat_infinite()`). If the pinned version differs, adjust these call sites; the public API in this plan does not change.

**Interfaces:**
- Produces:
  - `pub type SoundId = u32;`
  - `pub enum SoundFormat { Aiff, Ogg, Mod }` (derives `Clone, Copy, PartialEq, Eq, Debug`)
  - `pub struct AudioBackend` with:
    - `pub fn new(volume: u8) -> AudioBackend`
    - `pub fn play_tone(&mut self, freq_hz: f32, ms: u32, z_volume: u8)`
    - `pub fn stop(&mut self, id: SoundId)`
    - `pub fn stop_all(&mut self)`
    - `pub fn set_volume(&mut self, volume: u8)`
    - `pub fn finished(&mut self) -> Vec<SoundId>`
  - Free fns (crate-visible, tested): `fn gain(master: u8, z_volume: u8) -> f32`, `fn synth_tone(freq_hz: f32, ms: u32) -> Vec<f32>`
  - `play_sample` is declared as part of the public API but implemented in Task 5.

> **Volume rule (encode consistently across all tasks):** all playback volume is applied via `sink.set_volume(gain(master, z_volume))`. `synth_tone` produces a unit-amplitude decaying sine (the decay is its amplitude envelope). `set_volume(new_master)` updates `self.master` and calls `sink.set_volume(new_master as f32 / 100.0)` on every live sink (a master trim; the per-sound z_volume is not re-derived — documented, acceptable).

- [ ] **Step 1: Add the workspace member + crate manifest**

Edit `Cargo.toml` (root) `:2`:

```toml
members = ["crates/zvm", "crates/zvm-cli", "crates/mapper", "crates/app", "crates/blorb", "crates/gvm", "crates/gvm-cli"]
```
→
```toml
members = ["crates/zvm", "crates/zvm-cli", "crates/mapper", "crates/app", "crates/blorb", "crates/gvm", "crates/gvm-cli", "crates/audio"]
```

Create `crates/audio/Cargo.toml`:

```toml
[package]
name = "audio"
version = "0.1.0"
edition = "2021"

[features]
default = ["playback", "mod-music"]
playback = ["dep:rodio"]
mod-music = ["dep:mod_player"]

[dependencies]
rodio = { version = "0.19", optional = true }
mod_player = { version = "0.1", optional = true }
```

- [ ] **Step 2: Write the failing tests**

Create `crates/audio/src/lib.rs` with ONLY the test module first (so the crate compiles as a lib target with a failing test):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_combines_master_and_z_volume() {
        assert_eq!(gain(100, 8), 1.0);        // full master, loudest z-scale
        assert_eq!(gain(50, 8), 0.5);         // half master
        assert_eq!(gain(100, 4), 0.5);        // z 4/8
        assert_eq!(gain(100, 0), 1.0);        // 0 -> treated as full
        assert_eq!(gain(100, 255), 1.0);      // 255 -> loudest
        assert_eq!(gain(0, 8), 0.0);          // muted master
    }

    #[test]
    fn synth_tone_has_expected_length_and_energy() {
        let s = synth_tone(440.0, 100); // 100ms @ 44100 = 4410 samples
        assert_eq!(s.len(), 4410, "length = ms/1000 * 44100");
        assert!(s.iter().any(|v| v.abs() > 0.1), "tone must not be silent");
        // Decaying envelope: the first quarter is louder than the last quarter.
        let peak_early = s[..1000].iter().fold(0.0_f32, |a, v| a.max(v.abs()));
        let peak_late = s[3410..].iter().fold(0.0_f32, |a, v| a.max(v.abs()));
        assert!(peak_early > peak_late, "amplitude decays over time");
    }

    #[test]
    fn backend_no_device_paths_never_panic() {
        // Constructing a backend must succeed even with no output device (CI).
        let mut b = AudioBackend::new(100);
        b.play_tone(800.0, 50, 8);
        b.set_volume(50);
        b.stop(1);
        b.stop_all();
        let _ = b.finished();
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p audio`
Expected: FAIL — `cannot find function gain` / `synth_tone` / `AudioBackend`.

- [ ] **Step 4: Implement the crate (tones + registry; both cfg impls)**

Prepend the implementation above the test module in `crates/audio/src/lib.rs`:

```rust
//! Cross-platform host-side audio backend for babelmap. Plays synthesised tones
//! (Z-machine bleeps) and decoded samples (Blorb `Snd ` resources) via `rodio`.
//! With the `playback` feature off, the backend is a compile-time no-op.

/// Identifies a playing sampled sound so the host can stop it or detect its end.
pub type SoundId = u32;

/// Sampled-sound container format, chosen by the host from the Blorb chunk type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SoundFormat {
    Aiff,
    Ogg,
    Mod,
}

const SAMPLE_RATE: u32 = 44100;

/// Master+Z-scale gain in 0.0..=1.0. Master is 0..=100; z_volume is the Z-machine
/// 1..=8 scale, with 0/255 meaning "loudest" (full).
fn gain(master: u8, z_volume: u8) -> f32 {
    (master.min(100) as f32 / 100.0)
        * match z_volume {
            0 | 255 => 1.0,
            v => (v.min(8) as f32) / 8.0,
        }
}

/// A short decaying sine at `freq_hz` for `ms` at 44100 Hz (unit amplitude,
/// linear decay envelope). Volume is applied by the caller via the sink.
fn synth_tone(freq_hz: f32, ms: u32) -> Vec<f32> {
    let n = ((ms as f32 / 1000.0) * SAMPLE_RATE as f32) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let env = (1.0 - (i as f32 / n.max(1) as f32)).max(0.0);
        let s = (2.0 * std::f32::consts::PI * freq_hz * t).sin();
        out.push(s * env);
    }
    out
}

// ── Real backend (playback feature on) ────────────────────────────────────────

#[cfg(feature = "playback")]
pub struct AudioBackend {
    stream: Option<(rodio::OutputStream, rodio::OutputStreamHandle)>,
    samples: std::collections::HashMap<SoundId, rodio::Sink>,
    tones: Vec<rodio::Sink>,
    next_id: SoundId,
    master: u8,
}

#[cfg(feature = "playback")]
impl AudioBackend {
    pub fn new(volume: u8) -> AudioBackend {
        let stream = match rodio::OutputStream::try_default() {
            Ok((s, h)) => Some((s, h)),
            Err(e) => {
                eprintln!("audio: no output device ({e}); sound disabled");
                None
            }
        };
        AudioBackend {
            stream,
            samples: std::collections::HashMap::new(),
            tones: Vec::new(),
            next_id: 1,
            master: volume.min(100),
        }
    }

    pub fn play_tone(&mut self, freq_hz: f32, ms: u32, z_volume: u8) {
        let Some((_, handle)) = &self.stream else { return };
        let Ok(sink) = rodio::Sink::try_new(handle) else { return };
        sink.set_volume(gain(self.master, z_volume));
        sink.append(rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, synth_tone(freq_hz, ms)));
        self.tones.push(sink);
    }

    pub fn stop(&mut self, id: SoundId) {
        if let Some(sink) = self.samples.remove(&id) {
            sink.stop();
        }
    }

    pub fn stop_all(&mut self) {
        for (_, s) in self.samples.drain() {
            s.stop();
        }
        for s in self.tones.drain(..) {
            s.stop();
        }
    }

    pub fn set_volume(&mut self, volume: u8) {
        self.master = volume.min(100);
        let v = self.master as f32 / 100.0;
        for s in self.samples.values() {
            s.set_volume(v);
        }
        for s in &self.tones {
            s.set_volume(v);
        }
    }

    /// Drain completed sample ids (whose sink is empty) and prune finished tones.
    pub fn finished(&mut self) -> Vec<SoundId> {
        self.tones.retain(|s| !s.empty());
        let done: Vec<SoundId> = self
            .samples
            .iter()
            .filter(|(_, s)| s.empty())
            .map(|(id, _)| *id)
            .collect();
        for id in &done {
            self.samples.remove(id);
        }
        done
    }
}

// ── No-op backend (playback feature off) ──────────────────────────────────────

#[cfg(not(feature = "playback"))]
pub struct AudioBackend;

#[cfg(not(feature = "playback"))]
impl AudioBackend {
    pub fn new(_volume: u8) -> AudioBackend { AudioBackend }
    pub fn play_tone(&mut self, _freq_hz: f32, _ms: u32, _z_volume: u8) {}
    pub fn play_sample(&mut self, _bytes: &[u8], _format: SoundFormat, _z_volume: u8, _repeats: u8) -> Option<SoundId> { None }
    pub fn stop(&mut self, _id: SoundId) {}
    pub fn stop_all(&mut self) {}
    pub fn set_volume(&mut self, _volume: u8) {}
    pub fn finished(&mut self) -> Vec<SoundId> { Vec::new() }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p audio`
Expected: PASS (`gain_*`, `synth_tone_*`, `backend_no_device_paths_never_panic`).

Run: `cargo test -p audio --no-default-features`
Expected: PASS — the no-op backend compiles and the device-path test still runs (`gain`/`synth_tone` are always present).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/audio/Cargo.toml crates/audio/src/lib.rs
git commit -m "feat(audio): scaffold crate + tone synthesis backend

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Task 5: `crates/audio` — `play_sample` + AIFF/Ogg decode (Audio Task B)

**Files:**
- Modify: `crates/audio/src/lib.rs` (add `play_sample`, `decode_aiff`, `extended80_to_u32`, `append_samples`/`append_ogg` helpers; add tests)

**Interfaces:**
- Produces: `pub fn AudioBackend::play_sample(&mut self, bytes: &[u8], format: SoundFormat, z_volume: u8, repeats: u8) -> Option<SoundId>` on the real backend (the no-op backend already has the matching signature from Task 4).
- Consumes: `gain`, `SAMPLE_RATE`, `SoundFormat`, the `AudioBackend` registry from Task 4.

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `crates/audio/src/lib.rs`:

```rust
    /// Build a tiny 1-channel, 16-bit, 44100 Hz AIFF with two PCM frames.
    fn tiny_aiff() -> Vec<u8> {
        fn be_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(id);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 { v.push(0); }
            v
        }
        // COMM: channels=1, numFrames=2, sampleSize=16, rate = 44100 as 80-bit ext.
        let mut comm = Vec::new();
        comm.extend_from_slice(&1u16.to_be_bytes());       // channels
        comm.extend_from_slice(&2u32.to_be_bytes());       // numSampleFrames
        comm.extend_from_slice(&16u16.to_be_bytes());      // sampleSize
        comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]); // 44100
        // SSND: offset=0, blockSize=0, then two BE i16 samples: 256, -256.
        let mut ssnd = Vec::new();
        ssnd.extend_from_slice(&0u32.to_be_bytes());       // offset
        ssnd.extend_from_slice(&0u32.to_be_bytes());       // blockSize
        ssnd.extend_from_slice(&256i16.to_be_bytes());
        ssnd.extend_from_slice(&(-256i16).to_be_bytes());

        let mut body = Vec::new();
        body.extend_from_slice(b"AIFF");
        body.extend_from_slice(&be_chunk(b"COMM", &comm));
        body.extend_from_slice(&be_chunk(b"SSND", &ssnd));

        let mut form = Vec::new();
        form.extend_from_slice(b"FORM");
        form.extend_from_slice(&(body.len() as u32).to_be_bytes());
        form.extend_from_slice(&body);
        form
    }

    #[test]
    fn extended80_decodes_44100() {
        assert_eq!(extended80_to_u32(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]), 44100);
        assert_eq!(extended80_to_u32(&[0; 10]), 0);
    }

    #[test]
    fn decode_aiff_parses_comm_and_ssnd() {
        let (channels, rate, pcm) = decode_aiff(&tiny_aiff()).expect("valid AIFF");
        assert_eq!(channels, 1);
        assert_eq!(rate, 44100);
        assert_eq!(pcm, vec![256i16, -256i16]);
    }

    #[cfg(not(feature = "playback"))]
    #[test]
    fn play_sample_returns_none_without_playback() {
        let mut b = AudioBackend::new(100);
        assert!(b.play_sample(&tiny_aiff(), SoundFormat::Aiff, 8, 1).is_none());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p audio decode_aiff extended80`
Expected: FAIL — `cannot find function decode_aiff` / `extended80_to_u32`.

- [ ] **Step 3: Implement the decoders + `play_sample`**

Add these free functions to `crates/audio/src/lib.rs` (below `synth_tone`, outside any `#[cfg]`):

```rust
/// Decode a 10-byte 80-bit IEEE-754 extended float (AIFF sample rate) to u32 Hz.
/// Layout: 1 sign bit, 15 exponent bits (bias 16383), 64 mantissa bits with an
/// explicit integer bit. value = mantissa * 2^(exponent - 16383 - 63).
fn extended80_to_u32(b: &[u8; 10]) -> u32 {
    let exponent = (((b[0] & 0x7F) as u32) << 8) | b[1] as u32;
    let mantissa = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    if exponent == 0 && mantissa == 0 {
        return 0;
    }
    let e = exponent as i32 - 16383 - 63;
    let mut val = mantissa as f64;
    if e >= 0 {
        val *= 2f64.powi(e);
    } else {
        val /= 2f64.powi(-e);
    }
    val as u32
}

/// Parse an IFF `FORM`/`AIFF` container into (channels, sample_rate, interleaved
/// big-endian 16-bit PCM). Returns None on a malformed or non-16-bit AIFF.
fn decode_aiff(bytes: &[u8]) -> Option<(u16, u32, Vec<i16>)> {
    if bytes.len() < 12 || &bytes[0..4] != b"FORM" || &bytes[8..12] != b"AIFF" {
        return None;
    }
    let mut pos = 12;
    let mut channels: u16 = 0;
    let mut sample_rate: u32 = 0;
    let mut sample_size: u16 = 0;
    let mut pcm: Vec<i16> = Vec::new();
    while pos + 8 <= bytes.len() {
        let id = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
        let len = u32::from_be_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
        let data_start = pos + 8;
        if data_start + len > bytes.len() {
            break;
        }
        match &id {
            b"COMM" if len >= 18 => {
                channels = u16::from_be_bytes([bytes[data_start], bytes[data_start + 1]]);
                sample_size = u16::from_be_bytes([bytes[data_start + 6], bytes[data_start + 7]]);
                let mut ext = [0u8; 10];
                ext.copy_from_slice(&bytes[data_start + 8..data_start + 18]);
                sample_rate = extended80_to_u32(&ext);
            }
            b"SSND" if len >= 8 => {
                // Skip offset (u32) + blockSize (u32); the rest is big-endian PCM.
                let pcm_start = data_start + 8;
                let pcm_end = data_start + len;
                if sample_size <= 16 {
                    let mut i = pcm_start;
                    while i + 1 < pcm_end {
                        pcm.push(i16::from_be_bytes([bytes[i], bytes[i + 1]]));
                        i += 2;
                    }
                }
            }
            _ => {}
        }
        pos = data_start + len + (len & 1);
    }
    if channels == 0 || sample_rate == 0 || pcm.is_empty() {
        return None;
    }
    Some((channels, sample_rate, pcm))
}
```

Add `play_sample` to the real `impl AudioBackend` block (inside `#[cfg(feature = "playback")]`, e.g. right after `play_tone`):

```rust
    /// Decode `bytes` per `format`, play on a fresh sink at gain(master, z_volume),
    /// looping per `repeats` (0/255 = forever). Returns a SoundId to `stop`/track.
    /// Returns None if there is no device, the format is unsupported, or decode fails.
    pub fn play_sample(&mut self, bytes: &[u8], format: SoundFormat, z_volume: u8, repeats: u8) -> Option<SoundId> {
        use rodio::Source;
        let (_, handle) = self.stream.as_ref()?;
        let sink = rodio::Sink::try_new(handle).ok()?;
        sink.set_volume(gain(self.master, z_volume));
        let forever = repeats == 0 || repeats == 255;
        let count = repeats.max(1);
        match format {
            SoundFormat::Aiff => {
                let (channels, rate, pcm) = decode_aiff(bytes)?;
                if forever {
                    sink.append(rodio::buffer::SamplesBuffer::new(channels, rate, pcm.clone()).repeat_infinite());
                } else {
                    for _ in 0..count {
                        sink.append(rodio::buffer::SamplesBuffer::new(channels, rate, pcm.clone()));
                    }
                }
            }
            SoundFormat::Ogg => {
                if forever {
                    let dec = rodio::Decoder::new(std::io::Cursor::new(bytes.to_vec())).ok()?;
                    sink.append(dec.repeat_infinite());
                } else {
                    for _ in 0..count {
                        if let Ok(dec) = rodio::Decoder::new(std::io::Cursor::new(bytes.to_vec())) {
                            sink.append(dec);
                        }
                    }
                }
            }
            SoundFormat::Mod => {
                // MOD playback is added in the mod-music task; until then, unsupported.
                eprintln!("audio: MOD playback not yet supported");
                return None;
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.samples.insert(id, sink);
        Some(id)
    }
```

> Note: the `SoundFormat::Mod` arm above is fully self-contained (no `play_mod` reference, no `mod-music` cfg), so the crate builds green under DEFAULT features after this task. Task 6 REPLACES this arm with the feature-gated version that dispatches to `play_mod`. No cross-task build break.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p audio decode_aiff extended80`
Expected: PASS.

Run: `cargo build --workspace`
Expected: builds cleanly under default features (`playback` + `mod-music`) — the Mod arm is self-contained.

Run: `cargo test -p audio --no-default-features` (no `playback`)
Expected: PASS — `play_sample_returns_none_without_playback`.

- [ ] **Step 5: Commit**

```bash
git add crates/audio/src/lib.rs
git commit -m "feat(audio): play_sample with in-crate AIFF decoder + Ogg via rodio

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Task 6: `crates/audio` — MOD playback (Audio Task C)

**Files:**
- Modify: `crates/audio/src/lib.rs` (add `ModSource`, `play_mod`, and a MOD test — all behind `#[cfg(feature = "mod-music")]`)

> **Version note:** verify `mod_player` 0.1's API with `cargo doc -p mod_player --open` (or read `~/.cargo/registry/.../mod_player-0.1*/src/lib.rs`). This plan assumes: `mod_player::read_mod_file_slice(&[u8]) -> mod_player::Song`, `mod_player::PlayerState::new(channels: u32, sample_rate: u32) -> PlayerState`, and `mod_player::next_sample(&Song, &mut PlayerState) -> (f32, f32)`. If names differ, adjust `play_mod`/`ModSource` accordingly; the surrounding integration does not change.

**Interfaces:**
- Produces (crate-internal, `mod-music` only): `struct ModSource` implementing `rodio::Source<Item = f32> + Iterator<Item = f32>` with `channels() == 2`; `fn AudioBackend::play_mod(&mut self, bytes: &[u8], z_volume: u8) -> Option<SoundId>`.

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block (guarded so it only runs with the feature):

```rust
    /// Smallest structurally-valid 4-channel ("M.K.") ProTracker MOD: 1084-byte
    /// header (20-byte title, 31 * 30-byte sample records, songlen, restart,
    /// 128-byte order table, "M.K."), then one 1024-byte (64-row * 4-ch * 4-byte)
    /// zero pattern, and no sample data (all sample lengths are 0). It is silent,
    /// so the test asserts structure (channels == 2) + no-panic frame pulls, per
    /// the plan's documented fallback (a byte-exact non-silent tiny MOD is not
    /// practical to hand-build here).
    #[cfg(feature = "mod-music")]
    fn minimal_mod() -> Vec<u8> {
        let mut v = vec![0u8; 20];          // title
        v.extend(std::iter::repeat(0u8).take(31 * 30)); // 31 sample records
        v.push(1);                          // song length = 1 pattern in the order
        v.push(127);                        // restart position
        v.extend(std::iter::repeat(0u8).take(128)); // order table (pattern 0)
        v.extend_from_slice(b"M.K.");       // 4-channel tag
        v.extend(std::iter::repeat(0u8).take(64 * 4 * 4)); // one zero pattern
        v
    }

    #[cfg(feature = "mod-music")]
    #[test]
    fn mod_source_reports_stereo_and_pulls_frames() {
        use rodio::Source;
        let song = mod_player::read_mod_file_slice(&minimal_mod());
        let mut src = ModSource {
            song,
            state: mod_player::PlayerState::new(2, SAMPLE_RATE),
            rate: SAMPLE_RATE,
            pending_right: None,
        };
        assert_eq!(src.channels(), 2, "MOD is decoded as stereo");
        assert_eq!(src.sample_rate(), SAMPLE_RATE);
        for _ in 0..16 {
            assert!(src.next().is_some(), "frames pull without panic");
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p audio mod_source_reports_stereo`
Expected: FAIL — `cannot find type ModSource` / `cannot find struct ... pending_right`.

- [ ] **Step 3: Implement `ModSource` + `play_mod`**

Add to `crates/audio/src/lib.rs` (below the decoders; all behind the feature):

```rust
/// A `rodio::Source` that streams a ProTracker module via `mod_player`, yielding
/// interleaved stereo f32 (left then right of each frame on alternate `next`).
#[cfg(feature = "mod-music")]
struct ModSource {
    song: mod_player::Song,
    state: mod_player::PlayerState,
    rate: u32,
    pending_right: Option<f32>,
}

#[cfg(feature = "mod-music")]
impl Iterator for ModSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if let Some(r) = self.pending_right.take() {
            return Some(r);
        }
        let (l, r) = mod_player::next_sample(&self.song, &mut self.state);
        self.pending_right = Some(r);
        Some(l)
    }
}

#[cfg(feature = "mod-music")]
impl rodio::Source for ModSource {
    fn current_frame_len(&self) -> Option<usize> { None }
    fn channels(&self) -> u16 { 2 }
    fn sample_rate(&self) -> u32 { self.rate }
    fn total_duration(&self) -> Option<std::time::Duration> { None }
}
```

Add the private helper to the real `impl AudioBackend` block (feature-gated):

```rust
    #[cfg(feature = "mod-music")]
    fn play_mod(&mut self, bytes: &[u8], z_volume: u8) -> Option<SoundId> {
        let (_, handle) = self.stream.as_ref()?;
        let sink = rodio::Sink::try_new(handle).ok()?;
        sink.set_volume(gain(self.master, z_volume));
        let source = ModSource {
            song: mod_player::read_mod_file_slice(bytes),
            state: mod_player::PlayerState::new(2, SAMPLE_RATE),
            rate: SAMPLE_RATE,
            pending_right: None,
        };
        sink.append(source);
        let id = self.next_id;
        self.next_id += 1;
        self.samples.insert(id, sink);
        Some(id)
    }
```

- [ ] **Step 4: Wire the `SoundFormat::Mod` arm in `play_sample` to `play_mod`**

Replace the self-contained `SoundFormat::Mod` arm that Task 5 added to `play_sample` (in the real `impl AudioBackend`):

```rust
            SoundFormat::Mod => {
                // MOD playback is added in the mod-music task; until then, unsupported.
                eprintln!("audio: MOD playback not yet supported");
                return None;
            }
```

with the feature-gated version:

```rust
            SoundFormat::Mod => {
                #[cfg(feature = "mod-music")]
                { return self.play_mod(bytes, z_volume); }
                #[cfg(not(feature = "mod-music"))]
                {
                    eprintln!("audio: unsupported sound format (MOD; mod-music feature off)");
                    return None;
                }
            }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p audio`
Expected: PASS — `mod_source_reports_stereo_and_pulls_frames` plus all earlier audio tests, with default features (`playback` + `mod-music`).

Run: `cargo build -p audio --no-default-features --features playback`
Expected: builds — the MOD arm's `#[cfg(not(feature = "mod-music"))]` branch logs unsupported and returns None.

- [ ] **Step 6: Commit**

```bash
git add crates/audio/src/lib.rs
git commit -m "feat(audio): MOD playback via mod_player + ModSource

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Task 7: `crates/app` — config surface (App Task 1, no audio calls)

**Files:**
- Modify: `crates/app/src/config.rs` (default fns :183; struct fields :387; `Default` impl :419; `resolve` merge :486; `write_config` :544; test literal :840; roundtrip test :994)
- Modify: `crates/app/src/slash.rs` (add two commands after :161; bump count :610)
- Modify: `crates/app/src/input.rs` (`Action` variants :149; handlers :2059; `config_toggle_or_edit` :3501; `config_cycle` :3537)
- Modify: `crates/app/src/render/config_screen.rs` (`CONFIG_ROWS` :24; `ConfigRowKind` :31; `config_row_value` :173)

**Interfaces:**
- Produces:
  - `Config.enable_sound: bool` (default true), `Config.volume: u8` (default 100)
  - `crate::input::Action::ToggleSound`, `crate::input::Action::SetVolume(u8)`
  - slash commands `toggle-sound`, `volume <0-100>`
  - `ConfigRowKind::Num`
- Consumes: config `Config` derives `Clone`/`Serialize`/`Deserialize` (existing).

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `crates/app/src/config.rs` (after `honor_timed_input_defaults_true` at :994):

```rust
    #[test]
    fn enable_sound_defaults_true() {
        assert!(Config::default().enable_sound);
        let back: Config = toml::from_str("").unwrap();
        assert!(back.enable_sound, "absent key keeps default true");
        let off: Config = toml::from_str("enable_sound = false\n").unwrap();
        assert!(!off.enable_sound);
    }

    #[test]
    fn volume_defaults_100_and_roundtrips() {
        assert_eq!(Config::default().volume, 100);
        let back: Config = toml::from_str("").unwrap();
        assert_eq!(back.volume, 100, "absent key keeps default 100");
        let set: Config = toml::from_str("volume = 40\n").unwrap();
        assert_eq!(set.volume, 40);
    }
```

Add to `crates/app/src/slash.rs` tests (inside the module holding the count assertion, e.g. after :610):

```rust
    #[test]
    fn sound_commands_present() {
        let by = |n: &str| COMMANDS.iter().find(|c| c.name == n).expect(n);
        assert_eq!(by("toggle-sound").category, Category::Game);
        assert_eq!(by("volume").category, Category::Game);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p app enable_sound_defaults_true volume_defaults sound_commands_present`
Expected: FAIL — `no field enable_sound` / `no field volume` / `expect("toggle-sound")` panic.

- [ ] **Step 3: Implement the config fields**

In `crates/app/src/config.rs`, add default fns after `:184`:

```rust
fn default_enable_sound() -> bool { true }
fn default_volume() -> u8 { 100 }
```

Add struct fields after the `interpreter_number` field (before the struct's closing `}` at `:388`):

```rust
    /// When true (default), play audio for `sound_effect` (bleeps + Blorb samples).
    #[serde(default = "default_enable_sound")]
    pub enable_sound: bool,
    /// Master audio volume 0..=100 (default 100). Combined with the game's per-sound
    /// Z-scale volume.
    #[serde(default = "default_volume")]
    pub volume: u8,
```

In `impl Default for Config` add after `interpreter_number: None,` (`:419`):

```rust
            enable_sound: default_enable_sound(),
            volume: default_volume(),
```

In `resolve`, add to the merge block after `cfg.interpreter_number = from_file.interpreter_number;` (`:482`):

```rust
            cfg.enable_sound = from_file.enable_sound;
            cfg.volume = from_file.volume;
```

In `write_config`, add after `doc["honor_timed_input"] = toml_edit::value(cfg.honor_timed_input);` (`:539`):

```rust
    doc["enable_sound"] = toml_edit::value(cfg.enable_sound);
    doc["volume"] = toml_edit::value(cfg.volume as i64);
```

In the `write_config` test literal at `:840` (the `honor_timed_input: true,` / `interpreter_number: None,` block near the `Config { ... }` used by `write_config`), add after `interpreter_number: None,`:

```rust
            enable_sound: true,
            volume: 100,
```

- [ ] **Step 4: Implement the slash commands**

In `crates/app/src/slash.rs`, add after the `toggle-timed-input` command (`:161`, before the `// ── Map ──` comment):

```rust
    CommandSpec { name: "toggle-sound", category: Category::Game, context: Context::Global,
        usage: "toggle-sound", description: "toggle audio playback (bleeps + sampled sounds)",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleSound) },
    CommandSpec { name: "volume", category: Category::Game, context: Context::Global,
        usage: "volume <0-100>", description: "set the master audio volume (0-100)",
        dispatch: |a| match a.first().and_then(|s| s.parse::<u8>().ok()) {
            Some(v) if v <= 100 => SlashOutcome::Action(crate::input::Action::SetVolume(v)),
            _ => err("volume requires an integer 0-100 (e.g. volume 60)"),
        } },
```

Bump the count assertion at `:610`:

```rust
        assert_eq!(COMMANDS.len(), 49, "registry must match the spec's Full command table");
```
→
```rust
        assert_eq!(COMMANDS.len(), 51, "registry must match the spec's Full command table");
```

- [ ] **Step 5: Implement the input actions + handlers**

In `crates/app/src/input.rs`, add to the `Action` enum after `ToggleTimedInput` (`:149`):

```rust
    /// Toggle audio playback (config.enable_sound).
    ToggleSound,
    /// Set the master audio volume 0..=100 (config.volume).
    SetVolume(u8),
```

Add handlers in `apply_action` after the `Action::ToggleTimedInput` arm (`:2059`):

```rust
        Action::ToggleSound => {
            state.config.enable_sound = !state.config.enable_sound;
            state.set_status(if state.config.enable_sound { "sound on" } else { "sound off" });
        }
        Action::SetVolume(v) => {
            let v = v.min(100);
            state.config.volume = v;
            state.set_status(&format!("volume {v}"));
        }
```

> These handlers mutate only `state.config` here — this task compiles without the `audio` crate. Task 8 augments both handlers (and the config-screen `ConfigSave` hook) to propagate the change to `state.audio` so `/volume` scales live audio and `toggle-sound` off stops looping sounds.

- [ ] **Step 6: Implement the settings rows**

In `crates/app/src/render/config_screen.rs`, append to `CONFIG_ROWS` (after `("honor_timed_input", ConfigRowKind::Bool),` at `:24`):

```rust
    ("enable_sound",         ConfigRowKind::Bool),
    ("volume",               ConfigRowKind::Num),
```

Add a `Num` variant to `ConfigRowKind` (`:28-32`):

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigRowKind {
    Path,
    Bool,
    Enum,
    Num,
}
```

Extend `config_row_value` — replace the trailing `_ => String::new(),` (`:174`) with (indices 12/13):

```rust
        12 => bool_str(cfg.enable_sound),
        13 => cfg.volume.to_string(),
        _ => String::new(),
```

In `crates/app/src/input.rs`, extend `config_toggle_or_edit` — replace its trailing `_ => {}` at `:3502` with:

```rust
        12 => { if let Some(cs) = &mut state.config_screen { cs.working.enable_sound = !cs.working.enable_sound; } }
        _ => {}
```

Extend `config_cycle` — replace its trailing `_ => {}` at `:3538` with (volume steps by ±5, clamped):

```rust
        12 => working.enable_sound = !working.enable_sound,
        13 => working.volume = (working.volume as i32 + delta * 5).clamp(0, 100) as u8,
        _ => {}
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p app enable_sound_defaults_true volume_defaults sound_commands_present`
Expected: PASS.

Run: `cargo test -p app config_ registry_ config_screen`
Expected: PASS — including the `CONFIG_ROW_COUNT == CONFIG_ROWS.len()` regression (`input.rs:6712`) and the registry count.

Run: `cargo build -p app`
Expected: builds cleanly.

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/config.rs crates/app/src/slash.rs crates/app/src/input.rs crates/app/src/render/config_screen.rs
git commit -m "feat(app): enable_sound + volume config, toggle-sound/volume commands, settings rows

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Task 8: `crates/app` — sound-map + playback (App Task 2)

**Files:**
- Modify: `crates/app/Cargo.toml` (add `audio` dep)
- Modify: `crates/app/src/state.rs` (`AppState` fields near :849; `Default` init near :1103; `play_turn_sounds` method; `sound_kind_to_format` helper; test)
- Modify: `crates/app/src/main.rs` (call `state.play_turn_sounds` in `apply_turn_events` :4276; construct backend + resolve Blorb after `state.config = cfg;` :1248; add `resolve_sound_blorb` helper)
- Modify: `crates/app/src/input.rs` (augment the Task 7 `Action::SetVolume`/`Action::ToggleSound` handlers :2059 and the `Action::ConfigSave` hook :2992 to propagate runtime changes to `state.audio`)

**Interfaces:**
- Consumes: `audio::AudioBackend` (Tasks 4-6), `blorb::Blorb`/`blorb::SoundKind` (Task 1), `zvm::cpu::exec::SoundEvent` (Task 2). Also edits the `Action::SetVolume`/`Action::ToggleSound` handlers introduced in Task 7 and the `Action::ConfigSave` hook so live audio tracks `config.volume`/`config.enable_sound`.
- Produces:
  - `AppState.audio: Option<audio::AudioBackend>`
  - `AppState.sound_blorb: Option<blorb::Blorb>`
  - `AppState.sound_ids: std::collections::HashMap<u16, audio::SoundId>`
  - `AppState.sound_routines: std::collections::HashMap<audio::SoundId, u16>`
  - `pub fn AppState::play_turn_sounds(&mut self, sounds: &[zvm::cpu::exec::SoundEvent])`
  - `fn resolve_sound_blorb(story_path: &std::path::Path) -> Option<blorb::Blorb>` (in `main.rs`)

- [ ] **Step 1: Add the crate dependency**

Edit `crates/app/Cargo.toml`, add to `[dependencies]`:

```toml
audio = { path = "../audio" }
```

- [ ] **Step 2: Write the failing test**

Add to the `mod tests` block in `crates/app/src/state.rs` (near the `sound_pulse_defaults_none_and_holds_kind` test):

```rust
    #[test]
    fn play_turn_sounds_never_panics_without_device() {
        use zvm::cpu::exec::SoundEvent;
        let mut s = AppState::default();       // audio = None, sound_blorb = None
        s.config.enable_sound = true;
        // A #1 bleep event: play_turn_sounds must not panic with no backend.
        let ev = SoundEvent { number: 1, effect: 2, volume: 8, repeats: 0, routine: 0 };
        s.play_turn_sounds(&[ev]);             // no device -> silent, no panic
        // A #3 sampled start with no blorb loaded: no id remembered, no panic.
        let ev3 = SoundEvent { number: 3, effect: 2, volume: 8, repeats: 1, routine: 0 };
        s.play_turn_sounds(&[ev3]);
        assert!(s.sound_ids.is_empty(), "no sound id remembered without a blorb");
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p app play_turn_sounds_never_panics_without_device`
Expected: FAIL — `no method named play_turn_sounds` / `no field sound_ids`.

- [ ] **Step 4: Implement the AppState fields, helper, and method**

In `crates/app/src/state.rs`, add fields to `AppState` after `sound_pulse` (`:849`):

```rust
    /// Host audio backend, present when audio was enabled at launch. `None` when
    /// disabled or when construction was skipped.
    pub audio: Option<audio::AudioBackend>,
    /// The Blorb holding this story's `Snd ` resources, resolved at launch.
    pub sound_blorb: Option<blorb::Blorb>,
    /// Playing sampled sounds keyed by Z-machine sound number (for `effect` 3 stop).
    pub sound_ids: std::collections::HashMap<u16, audio::SoundId>,
    /// Finish-routines to fire when a sampled sound ends, keyed by its SoundId.
    pub sound_routines: std::collections::HashMap<audio::SoundId, u16>,
```

Add the inits in `impl Default for AppState` after `sound_pulse: None,` (`:1103`):

```rust
            audio: None,
            sound_blorb: None,
            sound_ids: std::collections::HashMap::new(),
            sound_routines: std::collections::HashMap::new(),
```

Add a free helper near the top of `crates/app/src/state.rs` (module scope) — map Blorb kind → backend format:

```rust
/// Map a Blorb sound kind to a backend format, or None for unsupported kinds.
fn sound_kind_to_format(k: blorb::SoundKind) -> Option<audio::SoundFormat> {
    match k {
        blorb::SoundKind::Aiff => Some(audio::SoundFormat::Aiff),
        blorb::SoundKind::Ogg => Some(audio::SoundFormat::Ogg),
        blorb::SoundKind::Mod => Some(audio::SoundFormat::Mod),
        blorb::SoundKind::Other => None,
    }
}
```

Add the method in `impl AppState` (anywhere in the existing impl block):

```rust
    /// Play the turn's sound events through the backend (gated on config +
    /// backend availability). Bleeps (#1/#2) → tones; samples (#>=3) → Blorb
    /// resource playback, remembering the SoundId (and finish routine) per number.
    /// `effect`: 2/default = start, 3 = stop, 1 = prepare (no-op).
    pub fn play_turn_sounds(&mut self, sounds: &[zvm::cpu::exec::SoundEvent]) {
        if !self.config.enable_sound {
            return;
        }
        let Some(backend) = self.audio.as_mut() else { return };
        for ev in sounds {
            match ev.number {
                0 => {}
                1 | 2 => {
                    if ev.effect == 0 || ev.effect == 2 {
                        let freq = if ev.number == 1 { 800.0 } else { 400.0 };
                        backend.play_tone(freq, 150, ev.volume);
                    }
                }
                n => match ev.effect {
                    3 => {
                        if let Some(id) = self.sound_ids.remove(&n) {
                            backend.stop(id);
                        }
                    }
                    1 => {} // prepare: decode on start
                    _ => {
                        if let Some(blorb) = &self.sound_blorb {
                            if let Some((bytes, kind)) = blorb.sound(n as u32) {
                                if let Some(fmt) = sound_kind_to_format(kind) {
                                    if let Some(id) = backend.play_sample(bytes, fmt, ev.volume, ev.repeats) {
                                        self.sound_ids.insert(n, id);
                                        if ev.routine != 0 {
                                            self.sound_routines.insert(id, ev.routine);
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }
    }
```

- [ ] **Step 5: Wire the backend + Blorb into `main.rs` and call playback**

In `crates/app/src/main.rs`, add a helper near the other free helpers (e.g. above `apply_turn_events`):

```rust
/// Resolve the sound-resource Blorb at launch: the story file itself if it is a
/// Blorb, else a sibling `<story>.blb` / `<story>.blorb`. `None` when no
/// container is found or it fails to parse.
fn resolve_sound_blorb(story_path: &std::path::Path) -> Option<blorb::Blorb> {
    if let Ok(bytes) = std::fs::read(story_path) {
        if blorb::Blorb::is_blorb(&bytes) {
            if let Ok(b) = blorb::Blorb::parse(bytes) {
                return Some(b);
            }
        }
    }
    for ext in ["blb", "blorb"] {
        let cand = story_path.with_extension(ext);
        if cand.exists() {
            if let Ok(bytes) = std::fs::read(&cand) {
                if let Ok(b) = blorb::Blorb::parse(bytes) {
                    return Some(b);
                }
            }
        }
    }
    None
}
```

After `state.config = cfg;` (`crates/app/src/main.rs:1248`), add:

```rust
    // Resolve the sound container + construct the audio backend (silent if the
    // feature is off, there is no device, or sound is disabled in config).
    state.sound_blorb = resolve_sound_blorb(&story_path);
    if state.config.enable_sound {
        state.audio = Some(audio::AudioBackend::new(state.config.volume));
    }
```

In `apply_turn_events`, after the border-pulse block added in Task 2 and before `state.loc_method = ...` (`crates/app/src/main.rs:4276`), add:

```rust
    // Audio is additive on top of the border pulse; gated inside play_turn_sounds.
    state.play_turn_sounds(&result.sounds);
```

- [ ] **Step 6: Propagate runtime volume/enable changes to the backend**

The Task 7 `Action::SetVolume`/`Action::ToggleSound` handlers and the `Action::ConfigSave` hook currently mutate only `state.config`; without this step `/volume`, `toggle-sound`, and the settings rows do not affect live audio (the backend's master is set once at launch). We use the **explicit-hook approach**: edit each mutation site to push the new value into `state.audio` (chosen over a per-turn sync in `play_turn_sounds` because it touches audio only when the user actually changes a setting, needs no `set_volume` no-op guard, and keeps `play_turn_sounds` a pure playback path).

In `crates/app/src/input.rs`, replace the two handlers added in Task 7 (`:2059`):

```rust
        Action::ToggleSound => {
            state.config.enable_sound = !state.config.enable_sound;
            state.set_status(if state.config.enable_sound { "sound on" } else { "sound off" });
        }
        Action::SetVolume(v) => {
            let v = v.min(100);
            state.config.volume = v;
            state.set_status(&format!("volume {v}"));
        }
```

with the versions that also drive the backend:

```rust
        Action::ToggleSound => {
            state.config.enable_sound = !state.config.enable_sound;
            state.set_status(if state.config.enable_sound { "sound on" } else { "sound off" });
            if !state.config.enable_sound {
                if let Some(b) = state.audio.as_mut() { b.stop_all(); }
            }
        }
        Action::SetVolume(v) => {
            let v = v.min(100);
            state.config.volume = v;
            state.set_status(&format!("volume {v}"));
            if let Some(b) = state.audio.as_mut() { b.set_volume(v); }
        }
```

In the `Action::ConfigSave` arm, right after `state.config = clone_config(&cs.working);` (`crates/app/src/input.rs:2994`), add:

```rust
                if let Some(b) = state.audio.as_mut() {
                    b.set_volume(state.config.volume);
                    if !state.config.enable_sound { b.stop_all(); }
                }
```

- [ ] **Step 7: Run tests + build to verify green**

Run: `cargo test -p app play_turn_sounds_never_panics_without_device`
Expected: PASS.

Run: `cargo build -p app`
Expected: builds cleanly (app now links `audio`; the two handlers + `ConfigSave` reach `state.audio`).

- [ ] **Step 8: Commit**

```bash
git add crates/app/Cargo.toml crates/app/src/state.rs crates/app/src/main.rs crates/app/src/input.rs
git commit -m "feat(app): resolve sound Blorb + AudioBackend, play turn sounds, live volume

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Task 9: `crates/app` — finish-routine callback (App Task 3)

**Files:**
- Modify: `crates/app/src/session.rs` (add `run_sound_finish` after `run_timed_interrupt` :236)
- Modify: `crates/app/src/main.rs` (poll completions in the `!event_ready` branch :1548-1571; clamp `poll_ms` when a sound is active :1531-1538)

**Interfaces:**
- Produces: `pub fn GameSession::run_sound_finish(&mut self, routine: u16) -> TurnResult`
- Consumes: `Machine::run_routine` (Task 3), `AudioBackend::finished` (Task 4), `AppState.sound_routines` (Task 8), `apply_game_driven_result` + `zvm_session_opt_mut` (existing).

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block in `crates/app/src/session.rs`:

```rust
    #[test]
    fn run_sound_finish_returns_turn_result() {
        // Reuse the char-mode fixture: run_sound_finish drives run_routine then
        // collects a TurnResult without stepping the read forward. Passing a 0
        // (bad/no routine) still returns a well-formed TurnResult (no panic).
        let story = read_char_story_v5();
        let mut sess = GameSession::new(story, true, None).expect("GameSession::new failed");
        let r = sess.run_sound_finish(0);
        assert!(r.sounds.is_empty(), "no new sounds from a finish callback");
        assert!(!r.quit, "a no-op finish routine does not quit");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app run_sound_finish_returns_turn_result`
Expected: FAIL — `no method named run_sound_finish`.

- [ ] **Step 3: Implement the session passthrough**

In `crates/app/src/session.rs`, add after `run_timed_interrupt` (`:236`):

```rust
    /// Run a sampled sound's finish-routine (v5+) to completion and drain any
    /// output it produced. The return value is ignored (ZMSD §9.4 — it does not
    /// abort anything). Does not step a pending read forward.
    pub fn run_sound_finish(&mut self, routine: u16) -> TurnResult {
        self.machine.run_routine(routine);
        self.collect_turn()
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app run_sound_finish_returns_turn_result`
Expected: PASS.

- [ ] **Step 5: Poll completions in the run loop**

In `crates/app/src/main.rs`, extend the `poll_ms` clamp so completions are noticed promptly. Replace the `base_poll_ms` line (`:1531`):

```rust
        let base_poll_ms = if state.has_active_animation() { TIDY_POLL_MS } else { 50 };
```

with (clamp to a modest cadence while a tracked sound is playing):

```rust
        let sound_active = !state.sound_routines.is_empty();
        let base_poll_ms = if state.has_active_animation() || sound_active { TIDY_POLL_MS } else { 50 };
```

In the `if !event_ready {` branch, after the timed-interrupt block (i.e. after the closing `}` of the `if let Some(dl) = state.input_deadline {` block at `:1571`), add:

```rust
            // Poll for finished sampled sounds and fire their finish-routines.
            let done: Vec<u32> = state.audio.as_mut().map(|b| b.finished()).unwrap_or_default();
            for id in done {
                if let Some(routine) = state.sound_routines.remove(&id) {
                    // Forget the number->id mapping for this finished sound too.
                    state.sound_ids.retain(|_, v| *v != id);
                    if routine != 0 {
                        if let Some(zs) = zvm_session_opt_mut(&mut *session) {
                            let result = zs.run_sound_finish(routine);
                            if apply_game_driven_result(
                                &mut state, &mut mapper, &result, &save_dir, &ifid, last_panes.map,
                            ) {
                                break;
                            }
                        }
                    }
                }
            }
```

> Borrow note: `state.audio.as_mut()...finished()` returns an owned `Vec<u32>`; the `.map(...).unwrap_or_default()` drops the `state.audio` borrow before the loop, so the subsequent `state.sound_routines`/`apply_game_driven_result(&mut state, ...)` calls do not conflict.

- [ ] **Step 6: Run the workspace build + app tests**

Run: `cargo build -p app`
Expected: builds cleanly.

Run: `cargo test -p app`
Expected: PASS (existing suite + the new finish-callback test).

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/session.rs crates/app/src/main.rs
git commit -m "feat(app): fire sound finish-routines on completion poll

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Task 10: `crates/zvm-cli` — host wiring (`--no-sound` / `--volume` + playback)

**Files:**
- Modify: `crates/zvm-cli/Cargo.toml` (add `audio` dep)
- Modify: `crates/zvm-cli/src/main.rs` (`Args` :226-232; `parse_args` :234-251; add `parse_volume`; usage :588; gates :630/:650; `CliSound` struct; bell drain → play :662-668 [as rewritten in Task 2]; completion poll in `read_line_raw`/`read_char_input`; flag tests :844)

**Interfaces:**
- Produces: `Args.no_sound: bool`; `fn parse_volume(argv: &[String]) -> Option<u8>`; `struct CliSound { backend: audio::AudioBackend, blorb: Option<blorb::Blorb>, ids: HashMap<u16, audio::SoundId>, routines: HashMap<audio::SoundId, u16> }`; `fn poll_sound_finish(sound: Option<&mut CliSound>, machine: &mut Machine, view: &mut screen::ScreenView, is_tty: bool)`.
- Consumes: `audio::AudioBackend`, `blorb`, `Machine::run_routine`, `Machine::pending_sounds`.

- [ ] **Step 1: Add the dependency + write the failing test**

Edit `crates/zvm-cli/Cargo.toml`, add to `[dependencies]`:

```toml
audio = { path = "../audio" }
```

Add to the `mod tests` block in `crates/zvm-cli/src/main.rs` (after `parses_no_timed_input_flag` at :849):

```rust
    #[test]
    fn parses_no_sound_flag() {
        let a = parse_args(&["zvm-cli".into(), "--no-sound".into(), "g".into()]);
        assert!(a.no_sound);
        let b = parse_args(&["zvm-cli".into(), "g".into()]);
        assert!(!b.no_sound);
    }

    #[test]
    fn parse_volume_reads_flag() {
        assert_eq!(parse_volume(&["--volume".into(), "60".into(), "g".into()]), Some(60));
        assert_eq!(parse_volume(&["g".into()]), None);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zvm-cli parses_no_sound_flag parse_volume_reads_flag`
Expected: FAIL — `no field no_sound` / `cannot find function parse_volume`.

- [ ] **Step 3: Implement flags + parsing**

In `crates/zvm-cli/src/main.rs`, extend `Args` (`:226-232`):

```rust
struct Args {
    story: Option<String>,
    no_status: bool,
    no_aux: bool,
    no_more: bool,
    no_timed_input: bool,
    no_sound: bool,
}
```

Update `parse_args` (`:234-251`) — the init and a new match arm:

```rust
fn parse_args(argv: &[String]) -> Args {
    let mut a = Args { story: None, no_status: false, no_aux: false, no_more: false, no_timed_input: false, no_sound: false };
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--no-status" | "--lower-only" => a.no_status = true,
            "--no-aux" => a.no_aux = true,
            "--no-more" | "--no-page" => a.no_more = true,
            "--no-timed-input" => a.no_timed_input = true,
            "--no-sound" => a.no_sound = true,
            "--volume" => i += 1, // also skip the following value token
            "--no-game-colours" => {}
            "-I" | "--interpreter" => i += 1, // also skip the following value token
            s if !s.starts_with("--") && a.story.is_none() => a.story = Some(s.to_string()),
            _ => {}
        }
        i += 1;
    }
    a
}
```

Add `parse_volume` after `parse_interpreter` (`:269`):

```rust
/// Read the master volume from `--volume N` (0..=100). None when absent/invalid.
fn parse_volume(args: &[String]) -> Option<u8> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--volume" {
            return it.next().and_then(|v| v.parse::<u8>().ok()).map(|v| v.min(100));
        }
    }
    None
}
```

Update the usage string (`:587-590`):

```rust
        eprintln!(
            "Usage: {} [--no-status] [--no-aux] [--no-more] [--no-timed-input] [--no-game-colours] [--no-sound] [--volume <0-100>] <story-file>",
            argv[0]
        );
```

- [ ] **Step 4: Run flag tests to verify they pass**

Run: `cargo test -p zvm-cli parses_no_sound_flag parse_volume_reads_flag`
Expected: PASS.

- [ ] **Step 5: Construct the sound context + play on turns + poll completions**

Add a `CliSound` struct and helper near the top of `crates/zvm-cli/src/main.rs` (module scope, after the imports):

```rust
use std::collections::HashMap;

/// The CLI's owned audio state: backend + resolved Blorb + live sound tracking.
struct CliSound {
    backend: audio::AudioBackend,
    blorb: Option<blorb::Blorb>,
    ids: HashMap<u16, audio::SoundId>,
    routines: HashMap<audio::SoundId, u16>,
}

/// Resolve the sound Blorb: a sibling `<story>.blb` / `<story>.blorb` next to the
/// raw story file (a raw `.z3`/`.z5` carries no `Snd ` resources itself).
fn resolve_sound_blorb(story_path: &Path) -> Option<blorb::Blorb> {
    for ext in ["blb", "blorb"] {
        let cand = story_path.with_extension(ext);
        if cand.exists() {
            if let Ok(bytes) = fs::read(&cand) {
                if let Ok(b) = blorb::Blorb::parse(bytes) {
                    return Some(b);
                }
            }
        }
    }
    None
}

fn sound_kind_to_format(k: blorb::SoundKind) -> Option<audio::SoundFormat> {
    match k {
        blorb::SoundKind::Aiff => Some(audio::SoundFormat::Aiff),
        blorb::SoundKind::Ogg => Some(audio::SoundFormat::Ogg),
        blorb::SoundKind::Mod => Some(audio::SoundFormat::Mod),
        blorb::SoundKind::Other => None,
    }
}

/// Play a drained batch of `SoundEvent`s: bleeps (#1/#2) → tones; samples (#>=3)
/// → Blorb resource playback tracked by number, remembering finish routines.
fn play_cli_sounds(cs: &mut CliSound, events: &[zvm::cpu::exec::SoundEvent]) {
    for ev in events {
        match ev.number {
            0 => {}
            1 | 2 => {
                if ev.effect == 0 || ev.effect == 2 {
                    let freq = if ev.number == 1 { 800.0 } else { 400.0 };
                    cs.backend.play_tone(freq, 150, ev.volume);
                }
            }
            n => match ev.effect {
                3 => { if let Some(id) = cs.ids.remove(&n) { cs.backend.stop(id); } }
                1 => {}
                _ => {
                    if let Some(blorb) = &cs.blorb {
                        if let Some((bytes, kind)) = blorb.sound(n as u32) {
                            if let Some(fmt) = sound_kind_to_format(kind) {
                                if let Some(id) = cs.backend.play_sample(bytes, fmt, ev.volume, ev.repeats) {
                                    cs.ids.insert(n, id);
                                    if ev.routine != 0 { cs.routines.insert(id, ev.routine); }
                                }
                            }
                        }
                    }
                }
            },
        }
    }
}

/// Poll finished sampled sounds; run their finish-routines and reprint the frame.
fn poll_sound_finish(sound: Option<&mut CliSound>, machine: &mut Machine, view: &mut screen::ScreenView, is_tty: bool) {
    let Some(cs) = sound else { return };
    let done = cs.backend.finished();
    let mut ran = false;
    for id in done {
        if let Some(routine) = cs.routines.remove(&id) {
            cs.ids.retain(|_, v| *v != id);
            if routine != 0 {
                machine.run_routine(routine);
                ran = true;
            }
        }
    }
    if ran && is_tty {
        print!("{}", view.frame(machine));
        let _ = io::stdout().flush();
    }
}
```

In `main`, after `let timed = !args.no_timed_input;` (`:630`), add:

```rust
    let sound_enabled = !args.no_sound;
    let volume = parse_volume(&argv).unwrap_or(100);
```

After `aux_preload(&mut machine, &aux_file, args.no_aux);` (`:651`), construct the sound context:

```rust
    let mut sound: Option<CliSound> = if sound_enabled {
        Some(CliSound {
            backend: audio::AudioBackend::new(volume),
            blorb: resolve_sound_blorb(&story_path),
            ids: HashMap::new(),
            routines: HashMap::new(),
        })
    } else {
        None
    };
```

Rewrite the bell-drain block (the one added in Task 2 at `:662-671`) to both ring and play:

```rust
        // Bleeps + sampled sounds: drain the turn's sound events. Ring the bell for
        // #1/#2 (TTY only), and play audio when enabled.
        if !machine.pending_sounds.is_empty() {
            let events: Vec<zvm::cpu::exec::SoundEvent> = machine.pending_sounds.drain(..).collect();
            let beeps = events.iter().filter(|e| e.number == 1 || e.number == 2).count();
            if beeps > 0 {
                print!("{}", screen::bleep_bytes(beeps, stdout_is_tty));
                let _ = io::stdout().flush();
            }
            if let Some(cs) = sound.as_mut() {
                play_cli_sounds(cs, &events);
            }
        }
```

Thread completion polling into the two raw readers. Change the signature of `read_char_input` (`:333-338`) to add a `sound` parameter:

```rust
fn read_char_input(
    is_tty: bool,
    machine: &mut Machine,
    view: &mut screen::ScreenView,
    timeout: Option<(u16, u16)>,
    sound: &mut Option<CliSound>,
) -> (u8, Option<(u16, u16)>, bool) {
```

Inside its poll-timeout branch, after `print!("{}", view.frame(machine));` on timeout (`:353-355`), also poll sounds — replace the timeout body (`:346-356`) with:

```rust
            if !event::poll(std::time::Duration::from_millis(t as u64 * 100)).unwrap_or(false) {
                let _ = terminal::disable_raw_mode();
                let out = machine.run_timed_interrupt();
                poll_sound_finish(sound.as_mut(), machine, view, is_tty);
                let _ = terminal::enable_raw_mode();
                if out.aborted {
                    break (0u8, last_resize, true);
                }
                print!("{}", view.frame(machine));
                let _ = io::stdout().flush();
                continue;
            }
```

For the **untimed** read_char (no `timeout`), add a bounded sound-poll tick: replace the `let result = loop {` opening so an untimed read still wakes to poll. Insert at the top of the loop body, before the `if let Some((t, _)) = timeout {`:

```rust
        if timeout.is_none() && sound.as_ref().map_or(false, |cs| !cs.routines.is_empty()) {
            if !event::poll(std::time::Duration::from_millis(50)).unwrap_or(false) {
                let _ = terminal::disable_raw_mode();
                poll_sound_finish(sound.as_mut(), machine, view, is_tty);
                let _ = terminal::enable_raw_mode();
                continue;
            }
        }
```

Apply the identical `sound: &mut Option<CliSound>` parameter and the same two insertions to `read_line_raw` (`:429-463`): add the parameter to its signature, poll sounds after `run_timed_interrupt` in the timeout branch, and add the untimed sound-poll tick at the top of its `loop {`.

Update all call sites of `read_char_input` and `read_line_raw` in `main` to pass `&mut sound` (search: `read_char_input(` / `read_line_raw(`).

- [ ] **Step 6: Run tests + build**

Run: `cargo test -p zvm-cli`
Expected: PASS (flag tests + existing suite).

Run: `cargo build -p zvm-cli`
Expected: builds cleanly.

- [ ] **Step 7: Commit**

```bash
git add crates/zvm-cli/Cargo.toml crates/zvm-cli/src/main.rs
git commit -m "feat(zvm-cli): --no-sound/--volume, play sounds, poll finish routines

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Task 11: README + bookkeeping

**Files:**
- Modify: `README.md`

**Interfaces:** none (documentation).

- [ ] **Step 1: Verify the whole workspace is green (feature matrix)**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS across all crates with default features.

Run: `cargo build -p audio --no-default-features` and `cargo build -p audio --no-default-features --features playback`
Expected: both build (no-op backend; AIFF/Ogg without MOD).

- [ ] **Step 2: Document the audio feature in the README**

Add an audio section to `README.md` covering:
- Audio support: real tones for the Z-machine bleeps (#1/#2) and sampled playback (AIFF/Ogg/MOD) of Blorb `Snd ` resources (#≥3), in both the `app` TUI and `zvm-cli`. The border pulse remains as a complementary/accessibility cue and the only cue when sound is disabled.
- `crates/audio` features: `playback` (default on; `rodio`) and `mod-music` (default on; `mod_player` for ProTracker `.mod`). Disable `playback` for headless/CI builds — the backend becomes a compile-time no-op. No output device at runtime degrades to silent.
- **Linux build prerequisite** for the `playback` feature: ALSA dev headers — `libasound2-dev` (Debian/Ubuntu) / `alsa-lib-devel` (Fedora).
- `zvm-cli` flags: `--no-sound` (disable audio), `--volume <0-100>` (master volume).
- `app`: `enable_sound` (default true) + `volume` (default 100) config keys; slash commands `toggle-sound` and `volume <0-100>`; the Settings (F2) `enable_sound` / `volume` rows.
- Validation: The Lurking Horror (`stories/lurkinghorror-r219-s870912.z3` + sibling `stories/Lurking.blb`) plays its sampled sounds in both hosts.

Add these entries to the existing feature list / flags table / config documentation in the style already used in `README.md` (match the `honor_game_colours` / `--no-timed-input` entries' formatting).

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs(readme): document audio feature, flags, and ALSA build prereq

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Self-Review

**1. Spec coverage** (against `docs/superpowers/specs/2026-07-01-sound-zmachine-design.md`):

- §3 `crates/audio` backend (features, API, tones, samples, AIFF, Ogg, MOD, non-blocking, `finished`) → Tasks 4, 5, 6. ✔
- §4 `crates/blorb` `sound()` + `SoundKind` + host container resolution (self-Blorb or sibling `.blb`/`.blorb`) → Task 1 (accessor); resolution in Tasks 8 (app) + 10 (cli). ✔
- §5 `crates/zvm` `SoundEvent`/`pending_sounds` replacing `Beep`/`pending_beeps`; `sound_effect` records every call; border-pulse migration → Task 2. ✔
- §6 host wiring: construct backend from `enable_sound`+`volume`; drain `pending_sounds`; tone/sample/stop/prepare semantics; config keys + `toggle-sound`/`volume` slash + settings row; `--no-sound`/`--volume`; border pulse retained; **runtime `/volume`/`toggle-sound`/settings changes propagate to the live backend** (app Task 8 Step 6 augments the Task 7 handlers + `ConfigSave`; cli reads `--volume` at launch) → Tasks 7, 8, 10. ✔
- §7.1 finish-routine callback: `run_routine` extraction (Task 3), `finished()` (Task 4), host completion poll → `run_routine` (app Task 9, cli Task 10). ✔
- §7.2 `prepare` = no-op → encoded (effect 1 no-op) in Tasks 8 & 10. ✔
- §8 testing: AIFF parse + tone synth (Task 4/5), MOD fixture (Task 6), blorb `sound()` (Task 1), zvm `SoundEvent` + `run_routine` (Tasks 2/3). ✔
- §9 cross-platform notes + ALSA prereq → Global Constraints + Task 11. ✔
- §10 file/crate map → covered by the File Structure section + per-task file lists. ✔

No spec requirement is left without a task.

**2. Placeholder scan:** No "TBD"/"handle edge cases"/"similar to Task N"/"write tests for the above" placeholders. Every code step shows full code or an exact before→after edit with surrounding context. Task 5's `SoundFormat::Mod` arm is fully self-contained (logs unsupported + returns None, no `play_mod` reference, no `mod-music` cfg), so Task 5 leaves the tree green under DEFAULT features; Task 6 Step 4 upgrades that arm to the feature-gated `play_mod` dispatch. No cross-task build break, no non-default-feature workaround.

**3. Type/signature consistency:**
- `SoundEvent { number: u16, effect: u8, volume: u8, repeats: u8, routine: u16 }` — identical in the zvm definition (Task 2), engine tests (Task 2), app `play_turn_sounds` (Task 8), cli `play_cli_sounds` (Task 10). ✔
- `AudioBackend` methods: `new(u8)`, `play_tone(f32,u32,u8)`, `play_sample(&[u8],SoundFormat,u8,u8)->Option<SoundId>`, `stop(SoundId)`, `stop_all()`, `set_volume(u8)`, `finished()->Vec<SoundId>` — defined identically in both `#[cfg]` impls (Tasks 4-6) and called with matching argument types by app (Task 8) and cli (Task 10). ✔
- `SoundId = u32` — used as `HashMap<u16, u32>` (ids) and `HashMap<u32, u16>` (routines) consistently; the completion loop's `let done: Vec<u32>` matches `finished() -> Vec<SoundId>`. ✔
- `blorb::SoundKind` (Aiff/Ogg/Mod/Other) mapped to `audio::SoundFormat` (Aiff/Ogg/Mod) by identical `sound_kind_to_format` helpers in app (Task 8) and cli (Task 10). ✔
- `BeepKind { High, Low }` in `crate::state` — used by `SoundPulse.kind`, the state tests, the main.rs border render, and `apply_turn_events` (all Task 2). ✔
- `Machine::run_routine(&mut self, u16) -> u16` (Task 3) — called by `run_timed_interrupt` (Task 3), `GameSession::run_sound_finish` (Task 9), and `poll_sound_finish` (Task 10). ✔
- `TurnResult.sounds: Vec<SoundEvent>` — produced by `drain_turn` (Task 2), read by `apply_turn_events`/`play_turn_sounds` (Tasks 2/8), asserted in tests (Tasks 2/9). ✔
- Config `enable_sound: bool` / `volume: u8` — struct fields, defaults, `Default`, `resolve`, `write_config`, config-screen rows, and slash/input handlers all reference the same names/types (Task 7). ✔
- Runtime propagation (Task 8 Step 6): `Action::SetVolume` → `state.audio.set_volume(v)`; `Action::ToggleSound` (off) → `state.audio.stop_all()`; `Action::ConfigSave` → `set_volume(config.volume)` + conditional `stop_all()`. All call `AudioBackend` methods defined in Tasks 4-6 with matching types; `state.audio` field exists from Task 8 Step 4. ✔

**4. Green under default features (every task leaves the tree buildable with `cargo build --workspace` + `cargo test --workspace`):**
- Task 1 (blorb), Task 2 (zvm + host migration, full `cargo build --workspace`), Task 3 (zvm): default features, green. ✔
- Task 4 (audio scaffold): `cargo test -p audio` under default features; also verified `--no-default-features`. ✔
- Task 5 (`play_sample`): the `SoundFormat::Mod` arm is self-contained, so `cargo build --workspace` (default `playback` + `mod-music`) succeeds — the earlier non-default-feature-only workaround is removed. ✔ (Fix applied.)
- Task 6 (MOD): adds `ModSource`/`play_mod` then upgrades the Mod arm; `cargo test -p audio` under default features. ✔
- Tasks 7-9 (app): Task 7 compiles without the `audio` dep (config surface only); Task 8 adds the dep + fields + runtime propagation; Task 9 adds the finish poll — each ends with `cargo build -p app` + `cargo test -p app`. ✔
- Task 10 (cli): `cargo build -p zvm-cli` + `cargo test -p zvm-cli`. ✔
- Task 11: full-workspace + feature-matrix verification. ✔

All consistent; no gaps found. The two coordinator-reported defects (Task 5 default-feature build break; runtime volume/enable not reaching the backend) and the cosmetic test rename are fixed inline.
