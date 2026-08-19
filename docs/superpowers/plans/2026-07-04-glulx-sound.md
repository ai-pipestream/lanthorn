# Glk/Glulx Sound Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Play Glk sound channels for Glulx games — `glk_schannel_*` create/play/stop/set_volume plus `Evtype_SoundNotify` finish events — reusing the existing `audio` crate and blorb sound resolution.

**Architecture:** Channel *state* lives in `AppGlk` (persists across turns inside the gvm `Machine`); channel operations are buffered as an app-crate `SchannelOp` list, drained each turn into `TurnResult`, and played by `AppState` through the single shared `AudioBackend` so the existing volume/mute UI governs Glulx sound. gvm stays zero-dependency: it calls primitive-typed trait methods and delivers `Evtype_SoundNotify` by mirroring the existing Arrange-event injection.

**Tech Stack:** Rust workspace — `gvm` (Glulx VM, zero-dep), `app` (ratatui TUI), `audio` (rodio), `blorb`.

**Spec:** `docs/superpowers/specs/2026-07-04-glulx-sound-design.md`

## Global Constraints

- `zvm` and `gvm` crates stay **zero-dependency**; all audio decode/playback and the `SchannelOp` type live in the app crate. gvm trait methods take only primitive args.
- Cross-platform (Windows/Linux/macOS); no platform-specific code — the `audio` crate already abstracts the backend.
- README covers **major features only** — Glk/Glulx sound qualifies for a README note; per-title fixes do not.
- Every commit ends with these two trailers verbatim:
  - `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`
  - `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Never silently panic/quit on game-derived input; unknown channels resolve to safe defaults.
- Scope: Base Sound selectors `0x00F0–0x00FC` only (no Sound2). `gestalt_Sound2` (21) stays 0. Notify **is** implemented. `schannel_play` returns 1 optimistically (does not verify the resource exists).

**Build/test commands** (run from repo root `/Volumes/Videos/Source/lanthorn`):
- gvm tests: `cargo test -p gvm`
- audio tests: `cargo test -p audio`
- app tests: `cargo test -p app`
- Single test: `cargo test -p gvm <test_name> -- --exact` (or just `cargo test -p gvm <substr>`)

---

### Task 1: audio crate — linear-gain playback

**Files:**
- Modify: `crates/audio/src/lib.rs` (real backend ~211-347; no-op backend ~351-364; tests ~366+)

**Interfaces:**
- Produces:
  - `AudioBackend::play_sample_gain(&mut self, bytes: &[u8], format: SoundFormat, gain: f32, repeats: u8) -> Option<SoundId>`
  - `AudioBackend::set_sample_gain(&mut self, id: SoundId, gain: f32)`
  - `gain` is a pre-master fraction (0.0..=1.0, may exceed 1.0); final Sink volume = `master/100 * gain`.

**Background:** Today `samples: HashMap<SoundId, (Sink, u8)>` stores a Z-machine `z_volume: u8` per sound, and `set_volume` recomputes `gain(master, z_volume)`. Glk volume is linear `0..0x10000` and cannot ride the z-scale (where `0` means "full"). This task generalizes the per-sample volume slot to hold *either* a z-scale byte *or* a linear fraction, behavior-preserving for the Z-machine path, and adds the two linear-gain methods.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/audio/src/lib.rs`:

```rust
    #[test]
    fn vol_gain_linear_combines_with_master() {
        // Linear (Glk) per-sample gain multiplies master/100, independent of the
        // z-scale path.
        assert_eq!(vol_gain(100, SampleVol::Lin(1.0)), 1.0);
        assert_eq!(vol_gain(50, SampleVol::Lin(1.0)), 0.5);
        assert_eq!(vol_gain(100, SampleVol::Lin(0.5)), 0.5);
        assert_eq!(vol_gain(0, SampleVol::Lin(1.0)), 0.0);
    }

    #[test]
    fn vol_gain_z_matches_legacy_gain() {
        // The z-scale variant must equal the historical gain() for every input,
        // so the Z-machine path is byte-for-byte unchanged.
        for master in [0u8, 25, 50, 100] {
            for z in [0u8, 1, 4, 8, 255] {
                assert_eq!(vol_gain(master, SampleVol::Z(z)), gain(master, z));
            }
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p audio vol_gain`
Expected: FAIL — `cannot find function vol_gain` / `cannot find type SampleVol`.

- [ ] **Step 3: Add `SampleVol` + `vol_gain`, generalize the samples map**

In `crates/audio/src/lib.rs`, just above the real backend (near `gain` / `repeat_plan`, inside the `#[cfg(feature = "playback")]` world — place it right before `#[cfg(feature = "playback")] pub struct AudioBackend`):

```rust
/// Per-sample volume: either the Z-machine 1..=8 scale (`0`/`255` = full) or a
/// linear pre-master gain fraction (the Glk channel model). Stored per playing
/// sample so a later master-volume change re-applies the correct formula.
#[cfg(feature = "playback")]
#[derive(Clone, Copy)]
enum SampleVol {
    Z(u8),
    Lin(f32),
}

/// Final pre-output gain for a sample: `master/100` times the sample's own level.
#[cfg(feature = "playback")]
fn vol_gain(master: u8, v: SampleVol) -> f32 {
    match v {
        SampleVol::Z(z) => gain(master, z),
        SampleVol::Lin(f) => (master.min(100) as f32 / 100.0) * f.max(0.0),
    }
}
```

Change the samples map field type (real backend struct, ~216):

```rust
    samples: std::collections::HashMap<SoundId, (rodio::Sink, SampleVol)>,
```

- [ ] **Step 4: Extract a shared decode-to-sink helper and rewrite `play_sample`**

Replace the body of `play_sample` (~260-306) so the decode/append work is shared with the new gain method. Add a private helper and rewrite `play_sample` to store `SampleVol::Z`:

```rust
    /// Build a sink that will play `bytes` (decoded per `format`) `repeats` times.
    /// Volume is NOT set here — the caller applies it. Returns None if there is
    /// no device, the format is unsupported, or decode fails.
    fn build_sample_sink(&self, bytes: &[u8], format: SoundFormat, repeats: u8) -> Option<rodio::Sink> {
        use rodio::Source;
        let (_, handle) = self.stream.as_ref()?;
        let sink = rodio::Sink::try_new(handle).ok()?;
        let (forever, count) = repeat_plan(repeats);
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
                #[cfg(feature = "mod-music")]
                {
                    let source = mod_stream::ModSource::new(bytes, forever, count)?;
                    sink.append(source);
                }
                #[cfg(not(feature = "mod-music"))]
                {
                    eprintln!("audio: unsupported sound format (MOD; mod-music feature off)");
                    return None;
                }
            }
        }
        Some(sink)
    }

    pub fn play_sample(&mut self, bytes: &[u8], format: SoundFormat, z_volume: u8, repeats: u8) -> Option<SoundId> {
        let sink = self.build_sample_sink(bytes, format, repeats)?;
        sink.set_volume(vol_gain(self.master, SampleVol::Z(z_volume)));
        let id = self.next_id;
        self.next_id += 1;
        self.samples.insert(id, (sink, SampleVol::Z(z_volume)));
        Some(id)
    }

    /// Like `play_sample`, but with a linear pre-master `gain` fraction (the Glk
    /// channel volume model) instead of the Z-machine z-scale.
    pub fn play_sample_gain(&mut self, bytes: &[u8], format: SoundFormat, gain: f32, repeats: u8) -> Option<SoundId> {
        let sink = self.build_sample_sink(bytes, format, repeats)?;
        sink.set_volume(vol_gain(self.master, SampleVol::Lin(gain)));
        let id = self.next_id;
        self.next_id += 1;
        self.samples.insert(id, (sink, SampleVol::Lin(gain)));
        Some(id)
    }

    /// Set a live sample's linear pre-master gain (Glk `schannel_set_volume`).
    pub fn set_sample_gain(&mut self, id: SoundId, gain: f32) {
        if let Some((sink, v)) = self.samples.get_mut(&id) {
            *v = SampleVol::Lin(gain);
            sink.set_volume(vol_gain(self.master, SampleVol::Lin(gain)));
        }
    }
```

Update `set_volume` (~323-331) to use the stored `SampleVol`:

```rust
    pub fn set_volume(&mut self, volume: u8) {
        self.master = volume.min(100);
        for (s, v) in self.samples.values() {
            s.set_volume(vol_gain(self.master, *v));
        }
        for (s, z_volume) in &self.tones {
            s.set_volume(gain(self.master, *z_volume));
        }
    }
```

(`stop` and `finished` bind the tuple's second field with `_`, so they need no change.)

- [ ] **Step 5: Add matching no-op-backend stubs**

In the `#[cfg(not(feature = "playback"))] impl AudioBackend` block (~356-364), add:

```rust
    pub fn play_sample_gain(&mut self, _bytes: &[u8], _format: SoundFormat, _gain: f32, _repeats: u8) -> Option<SoundId> { None }
    pub fn set_sample_gain(&mut self, _id: SoundId, _gain: f32) {}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p audio`
Expected: PASS (new `vol_gain_*` tests plus all existing audio tests green).

- [ ] **Step 7: Commit**

```bash
git add crates/audio/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(audio): linear per-sample gain (play_sample_gain/set_sample_gain)

Quest: SQ-0208

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

### Task 2: gvm — sound gestalt + `sound_enabled`/`set_sound`

**Files:**
- Modify: `crates/gvm/src/exec.rs` — struct field (~143), constructor init (~275, in `with_glk`), `set_sound` (near `set_graphics` ~1725), `glk_gestalt` (~3128), tests (`mod tests`)

**Interfaces:**
- Produces: `Machine::set_sound(&mut self, on: bool)`; `glk_gestalt` returns 1 for selectors 8/9/10 when sound is enabled, 0 otherwise; 21 stays 0.

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block in `crates/gvm/src/exec.rs` (near the other `glk_gestalt_*` tests, e.g. after `glk_gestalt_reports_input_capabilities`):

```rust
    #[test]
    fn glk_gestalt_reports_sound_capabilities() {
        // gestalt_Sound(8)/SoundVolume(9)/SoundNotify(10) follow sound_enabled;
        // gestalt_Sound2(21) is never supported.
        let mut m = machine_with_glk(&[]);
        assert_eq!(m.glk_gestalt(8, 0), 0, "Sound off by default");
        assert_eq!(m.glk_gestalt(9, 0), 0);
        assert_eq!(m.glk_gestalt(10, 0), 0);
        m.set_sound(true);
        assert_eq!(m.glk_gestalt(8, 0), 1, "Sound supported once enabled");
        assert_eq!(m.glk_gestalt(9, 0), 1, "SoundVolume supported");
        assert_eq!(m.glk_gestalt(10, 0), 1, "SoundNotify supported");
        assert_eq!(m.glk_gestalt(21, 0), 0, "Sound2 never supported");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p gvm glk_gestalt_reports_sound_capabilities`
Expected: FAIL — `no method named set_sound` (and the enabled asserts would fail).

- [ ] **Step 3: Add the field, constructor init, and setter**

In the `Machine` struct, right after the `graphics_enabled` field (~143):

```rust
    /// Whether Glk sound channels are enabled (default false; hosts opt in).
    pub(crate) sound_enabled: bool,
```

In `with_glk` (the struct literal that initializes `graphics_enabled: false`, ~275), add alongside it:

```rust
            sound_enabled: false,
```

Add the setter right after `set_graphics` (~1727):

```rust
    /// Enable/disable Glk sound (gestalt + schannel opcodes).
    pub fn set_sound(&mut self, on: bool) {
        self.sound_enabled = on;
    }
```

- [ ] **Step 4: Add the gestalt arms**

In `glk_gestalt` (~3128), add after the graphics arms (after the `14 =>` line):

```rust
            8 => self.sound_enabled as u32,  // gestalt_Sound
            9 => self.sound_enabled as u32,  // gestalt_SoundVolume
            10 => self.sound_enabled as u32, // gestalt_SoundNotify
```

(Leave the `_ => 0` catch-all — it correctly returns 0 for gestalt_Sound2 (21) and everything else.)

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p gvm glk_gestalt`
Expected: PASS (the new test plus the existing gestalt tests).

- [ ] **Step 6: Commit**

```bash
git add crates/gvm/src/exec.rs
git commit -m "$(cat <<'EOF'
feat(gvm): advertise gestalt_Sound/SoundVolume/SoundNotify

Quest: SQ-0208

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

### Task 3: gvm — schannel dispatch → backend

**Files:**
- Modify: `crates/gvm/src/glk.rs` — `GlkBackend` trait (add methods ~320); `TestBackend` (struct ~338, `new` ~365, impl ~434) to record schannel calls
- Modify: `crates/gvm/src/exec.rs` — dispatch arms before the catch-all (~2765); tests
- Modify: `docs/superpowers/audits/2026-06-30-glulx-feature-gaps.md` — fix the swapped sound/hyperlink selector table

**Interfaces:**
- Consumes: `Machine::sound_enabled`, `set_sound` (Task 2).
- Produces (new `GlkBackend` methods, no-op defaults):
  - `schannel_create(&mut self, rock: u32) -> u32`
  - `schannel_destroy(&mut self, chan: u32)`
  - `schannel_iterate(&mut self, chan: u32) -> (u32, u32)` — returns `(next_ref, that_ref_rock)`
  - `schannel_get_rock(&mut self, chan: u32) -> u32`
  - `schannel_play(&mut self, chan: u32, snd: u32, repeats: u32, notify: u32) -> u32`
  - `schannel_stop(&mut self, chan: u32)`
  - `schannel_set_volume(&mut self, chan: u32, vol: u32)`
- Dispatch: selectors `0x00F0` create, `0x00F1` destroy, `0x00F2` iterate, `0x00F3` get_rock, `0x00F8` play, `0x00F9` play_ext, `0x00FA` stop, `0x00FB` set_volume, `0x00FC` load_hint (no-op). All gated on `sound_enabled`; return 0 when disabled.

- [ ] **Step 1: Add the trait methods (no-op defaults)**

In `crates/gvm/src/glk.rs`, in `pub trait GlkBackend`, right after `graphics_draw_image` (~320):

```rust
    /// Create a sound channel with rock `rock`; return its Glk ref (0 = failure).
    fn schannel_create(&mut self, _rock: u32) -> u32 { 0 }
    /// Destroy a sound channel.
    fn schannel_destroy(&mut self, _chan: u32) {}
    /// Iterate channels: `chan == 0` → first; else the channel after `chan`.
    /// Return `(next_ref_or_0, that_channel_rock_or_0)`.
    fn schannel_iterate(&mut self, _chan: u32) -> (u32, u32) { (0, 0) }
    /// The rock of channel `chan` (0 if unknown).
    fn schannel_get_rock(&mut self, _chan: u32) -> u32 { 0 }
    /// Play sound resource `snd` on `chan`, `repeats` times (0xFFFFFFFF = forever),
    /// posting an `Evtype_SoundNotify` with value `notify` on completion when
    /// `notify != 0`. Return 1 on success, 0 on failure.
    fn schannel_play(&mut self, _chan: u32, _snd: u32, _repeats: u32, _notify: u32) -> u32 { 0 }
    /// Stop whatever is playing on `chan` (no notify is posted for a stop).
    fn schannel_stop(&mut self, _chan: u32) {}
    /// Set `chan`'s volume (Glk scale: 0x10000 = full).
    fn schannel_set_volume(&mut self, _chan: u32, _vol: u32) {}
```

- [ ] **Step 2: Extend `TestBackend` to record schannel calls**

In `crates/gvm/src/glk.rs`, add fields to `struct TestBackend` (~354, after `backgrounds`):

```rust
    /// Next schannel ref to hand out (pre-incremented; first create → 1).
    next_schannel: u32,
    /// Rock per live schannel ref.
    schannel_rocks: BTreeMap<u32, u32>,
    /// Human-readable log of schannel calls, in order (for dispatch assertions).
    sound_log: Vec<String>,
```

Initialize them in `TestBackend::new` (~366, inside the struct literal):

```rust
            next_schannel: 0,
            schannel_rocks: BTreeMap::new(),
            sound_log: Vec::new(),
```

Add an accessor in `impl TestBackend` (near `draws`, ~427):

```rust
    /// The recorded schannel call log (create/play/stop/setvol/destroy), in order.
    pub fn sound_log(&self) -> &[String] {
        &self.sound_log
    }
```

Implement the methods in `impl GlkBackend for TestBackend` (place after the graphics methods, before `as_any`):

```rust
    fn schannel_create(&mut self, rock: u32) -> u32 {
        self.next_schannel += 1;
        let id = self.next_schannel;
        self.schannel_rocks.insert(id, rock);
        self.sound_log.push(format!("create rock={rock} -> {id}"));
        id
    }
    fn schannel_destroy(&mut self, chan: u32) {
        self.schannel_rocks.remove(&chan);
        self.sound_log.push(format!("destroy chan={chan}"));
    }
    fn schannel_iterate(&mut self, chan: u32) -> (u32, u32) {
        let next = if chan == 0 {
            self.schannel_rocks.keys().next().copied()
        } else {
            self.schannel_rocks.range((chan + 1)..).next().map(|(k, _)| *k)
        };
        match next {
            Some(id) => (id, *self.schannel_rocks.get(&id).unwrap_or(&0)),
            None => (0, 0),
        }
    }
    fn schannel_get_rock(&mut self, chan: u32) -> u32 {
        *self.schannel_rocks.get(&chan).unwrap_or(&0)
    }
    fn schannel_play(&mut self, chan: u32, snd: u32, repeats: u32, notify: u32) -> u32 {
        self.sound_log.push(format!("play chan={chan} snd={snd} repeats={repeats} notify={notify}"));
        1
    }
    fn schannel_stop(&mut self, chan: u32) {
        self.sound_log.push(format!("stop chan={chan}"));
    }
    fn schannel_set_volume(&mut self, chan: u32, vol: u32) {
        self.sound_log.push(format!("setvol chan={chan} vol={vol}"));
    }
```

- [ ] **Step 3: Write the failing dispatch tests**

Add to `mod tests` in `crates/gvm/src/exec.rs` (near the graphics dispatch tests). `backend_of` (~5338) and `glk_call`/`run_with_ram` already exist:

```rust
    #[test]
    fn schannel_dispatch_routes_to_backend_when_enabled() {
        use asm::Op::{C32, C8, Mem16};
        // create(rock=7)->0x100, get_rock(1)->0x104, play(1, snd=5)->0x108,
        // play_ext(1, snd=6, repeats=3, notify=9)->0x10C, set_volume(1, 0x8000),
        // stop(1), destroy(1).
        let mut body = glk_call(0xF0, &[C8(7)], Mem16(0x0100)); // schannel_create
        body.extend(glk_call(0xF3, &[C8(1)], Mem16(0x0104)));   // schannel_get_rock
        body.extend(glk_call(0xF8, &[C8(1), C8(5)], Mem16(0x0108))); // schannel_play
        body.extend(glk_call(0xF9, &[C8(1), C8(6), C8(3), C8(9)], Mem16(0x010C))); // play_ext
        body.extend(glk_call(0xFB, &[C8(1), C32(0x8000)], asm::Op::Zero)); // set_volume
        body.extend(glk_call(0xFA, &[C8(1)], asm::Op::Zero)); // stop
        body.extend(glk_call(0xF1, &[C8(1)], asm::Op::Zero)); // destroy
        body.extend(asm::ins(0x120, &[]));                    // quit
        let m = run_with_ram(body, 0x200, |m| m.set_sound(true));

        assert_eq!(m.mem.read32(0x0100).unwrap(), 1, "create returns the first channel ref");
        assert_eq!(m.mem.read32(0x0104).unwrap(), 7, "get_rock returns the stored rock");
        assert_eq!(m.mem.read32(0x0108).unwrap(), 1, "play returns success");
        assert_eq!(m.mem.read32(0x010C).unwrap(), 1, "play_ext returns success");

        let log = backend_of(&m).sound_log();
        assert!(log.iter().any(|l| l == "play chan=1 snd=5 repeats=1 notify=0"),
            "plain play forwards repeats=1 notify=0: {log:?}");
        assert!(log.iter().any(|l| l == "play chan=1 snd=6 repeats=3 notify=9"),
            "play_ext threads repeats+notify: {log:?}");
        assert!(log.iter().any(|l| l == "setvol chan=1 vol=32768"), "set_volume forwarded: {log:?}");
        assert!(log.iter().any(|l| l == "stop chan=1"), "stop forwarded: {log:?}");
        assert!(log.iter().any(|l| l == "destroy chan=1"), "destroy forwarded: {log:?}");
        assert!(!m.diagnostics.iter().any(|d| d.contains("unhandled")),
            "no unhandled-selector diagnostic: {:?}", m.diagnostics);
    }

    #[test]
    fn schannel_dispatch_is_inert_when_sound_disabled() {
        use asm::Op::{C8, Mem16};
        // With sound disabled, create returns 0 (NULL channel) and nothing is
        // recorded. (A spec-correct game won't call these — gestalt reports 0 —
        // but a probe must get a safe 0, not a diagnostic-spamming fallthrough.)
        let mut body = glk_call(0xF0, &[C8(7)], Mem16(0x0100)); // schannel_create
        body.extend(glk_call(0xFC, &[C8(5), C8(1)], asm::Op::Zero)); // sound_load_hint
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {}); // sound left disabled
        assert_eq!(m.mem.read32(0x0100).unwrap(), 0, "create returns NULL when sound is off");
        assert!(backend_of(&m).sound_log().is_empty(), "no backend calls when sound is off");
    }
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p gvm schannel_dispatch`
Expected: FAIL — the selectors currently hit the catch-all (`create` returns 0 even "enabled", `sound_log` empty, and an "unhandled selector" diagnostic is present).

- [ ] **Step 5: Add the dispatch arms**

In `crates/gvm/src/exec.rs` `glk_dispatch`, insert immediately before the `other =>` catch-all (~2766):

```rust
            // ── sound channels (Glk Sound; GLULX_NOTES) ─────────────────────────
            0x00F0 => {
                // glk_schannel_create(rock) -> chan
                if self.sound_enabled { self.backend.schannel_create(a(0)) } else { 0 }
            }
            0x00F1 => {
                // glk_schannel_destroy(chan)
                if self.sound_enabled { self.backend.schannel_destroy(a(0)); }
                0
            }
            0x00F2 => {
                // glk_schannel_iterate(chan, &rock) -> next chan
                if self.sound_enabled {
                    let (next, rock) = self.backend.schannel_iterate(a(0));
                    self.glk_store_ptr(a(1), rock)?;
                    next
                } else {
                    self.glk_store_ptr(a(1), 0)?;
                    0
                }
            }
            0x00F3 => {
                // glk_schannel_get_rock(chan) -> rock
                if self.sound_enabled { self.backend.schannel_get_rock(a(0)) } else { 0 }
            }
            0x00F8 => {
                // glk_schannel_play(chan, snd) -> 1/0  (repeats=1, no notify)
                if self.sound_enabled { self.backend.schannel_play(a(0), a(1), 1, 0) } else { 0 }
            }
            0x00F9 => {
                // glk_schannel_play_ext(chan, snd, repeats, notify) -> 1/0
                if self.sound_enabled { self.backend.schannel_play(a(0), a(1), a(2), a(3)) } else { 0 }
            }
            0x00FA => {
                // glk_schannel_stop(chan)
                if self.sound_enabled { self.backend.schannel_stop(a(0)); }
                0
            }
            0x00FB => {
                // glk_schannel_set_volume(chan, vol)
                if self.sound_enabled { self.backend.schannel_set_volume(a(0), a(1)); }
                0
            }
            0x00FC => {
                // glk_sound_load_hint(snd, flag) — decoding is on-demand; accept + ignore.
                0
            }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p gvm schannel_dispatch`
Expected: PASS (both dispatch tests).

- [ ] **Step 7: Fix the swapped selector table in the audit doc**

In `docs/superpowers/audits/2026-06-30-glulx-feature-gaps.md`, the sound and hyperlink selector ranges are swapped. Correct them: sound channels are `0x00F0–0x00FC` (Sound2 adds `0x00F4–0x00F7`, `0x00FD`); hyperlinks are `0x0100–0x0103`. Edit the two lines (~302, ~310) so the sound row reads `0x00F0–0x00FC` and the hyperlink row reads `0x0100–0x0103`. Add a short parenthetical: `(corrected 2026-07-04: was swapped)`.

- [ ] **Step 8: Run the full gvm suite + commit**

Run: `cargo test -p gvm`
Expected: PASS (all green).

```bash
git add crates/gvm/src/glk.rs crates/gvm/src/exec.rs docs/superpowers/audits/2026-06-30-glulx-feature-gaps.md
git commit -m "$(cat <<'EOF'
feat(gvm): dispatch Glk schannel selectors (0x00F0-0x00FC) to the backend

Quest: SQ-0208

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

### Task 4: gvm — `deliver_sound_notify`

**Files:**
- Modify: `crates/gvm/src/exec.rs` — new method near `deliver_arrange` (~3109); tests

**Interfaces:**
- Produces: `Machine::deliver_sound_notify(&mut self, sound: u32, notify: u32)` — writes `Evtype_SoundNotify{win:0, val1:sound, val2:notify}` into a suspended `glk_select`, or queues it (via the model event queue) when not suspended.
- Reuses: `GlkEvent`, `glk::evtype::SOUND_NOTIFY`, `write_event`, `pending_input`, `self.glk.push_event` (all existing).

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/gvm/src/exec.rs` (near `deliver_arrange_interrupts_suspended_select_without_consuming_request`, ~5816). `step_to_event`/`read_event`/`machine_ram`/`glk_call` already exist:

```rust
    #[test]
    fn deliver_sound_notify_writes_into_a_suspended_select() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event then select: the select suspends on the line request;
        // a sound-notify is written into it WITHOUT consuming the line request, so
        // the next select re-suspends on the still-pending read.
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select → suspend @0x100
        body.extend(glk_call(0xC0, &[C16(0x0110)], Zero)); // select again → re-suspend @0x110
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 }, "first select suspends");
        m.deliver_sound_notify(6, 42);
        // evtype_SoundNotify = 7, win = 0, val1 = sound (6), val2 = notify (42).
        assert_eq!(read_event(&m, 0x100), (7, 0, 6, 42), "sound-notify written to the suspended select");
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 }, "line request persisted across the notify");
    }

    #[test]
    fn deliver_sound_notify_queues_when_not_suspended() {
        use asm::Op::{C16, Zero};
        // With nothing waiting, a notify is queued and delivered by the NEXT select.
        let body = {
            let mut b = glk_call(0xC0, &[C16(0x0100)], Zero); // select → drains the queued event
            b.extend(asm::ins(0x120, &[]));
            b
        };
        let mut m = machine_ram(body, 0x200);
        m.deliver_sound_notify(3, 99); // not suspended yet → queue
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "select consumes the queued event and runs to quit");
        assert_eq!(read_event(&m, 0x100), (7, 0, 3, 99), "queued sound-notify delivered by the select");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p gvm deliver_sound_notify`
Expected: FAIL — `no method named deliver_sound_notify`.

- [ ] **Step 3: Implement `deliver_sound_notify`**

In `crates/gvm/src/exec.rs`, right after `deliver_arrange` (~3109):

```rust
    /// Deliver a Glk `Evtype_SoundNotify` for a finished sound: `sound` is the
    /// resource number, `notify` the value the game passed to
    /// `glk_schannel_play_ext`. Mirrors [`Machine::deliver_arrange`] — written
    /// directly into a suspended `glk_select` (without consuming the window's
    /// input request, so the game handles it and re-suspends), or queued for the
    /// next select when the VM is not currently blocked.
    pub fn deliver_sound_notify(&mut self, sound: u32, notify: u32) {
        let ev = GlkEvent { etype: glk::evtype::SOUND_NOTIFY, win: 0, val1: sound, val2: notify };
        if let Some(pi) = self.pending_input.take() {
            if let Err(e) = self.write_event(pi.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else {
            self.glk.push_event(ev);
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p gvm deliver_sound_notify`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/gvm/src/exec.rs
git commit -m "$(cat <<'EOF'
feat(gvm): deliver_sound_notify injects Evtype_SoundNotify

Quest: SQ-0208

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

### Task 5: app — `SchannelOp` + `AppGlk` channel state

**Files:**
- Modify: `crates/app/src/session.rs` — add `SchannelOp` enum (near `TranscriptElem`)
- Modify: `crates/app/src/glk_backend.rs` — `AppGlk` fields (~127), `with_graphics` init (~151), `impl GlkBackend for AppGlk` methods (~544+), `take_sound_ops`; tests

**Interfaces:**
- Consumes: the `GlkBackend` schannel method signatures (Task 3).
- Produces:
  - `pub enum SchannelOp { Play { chan, snd, repeats, notify, volume } (all u32), Stop { chan: u32 }, SetVolume { chan: u32, vol: u32 }, Destroy { chan: u32 } }` in `session.rs`.
  - `AppGlk::take_sound_ops(&mut self) -> Vec<crate::session::SchannelOp>`.

- [ ] **Step 1: Define `SchannelOp`**

In `crates/app/src/session.rs`, near `TranscriptElem` (before `TurnResult`):

```rust
/// One buffered Glk sound-channel operation, emitted by `AppGlk` during a turn
/// and drained into `TurnResult.glulx_sound_ops` for `AppState` to play. Channel
/// *state* (refs, rocks, volume) lives in `AppGlk`; only the playback-affecting
/// operations travel here. `Play.volume` snapshots the channel's current Glk
/// volume so the player (which cannot see `AppGlk`) can compute gain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchannelOp {
    Play { chan: u32, snd: u32, repeats: u32, notify: u32, volume: u32 },
    Stop { chan: u32 },
    SetVolume { chan: u32, vol: u32 },
    Destroy { chan: u32 },
}
```

- [ ] **Step 2: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/app/src/glk_backend.rs`:

```rust
    #[test]
    fn appglk_schannel_create_allocates_refs_and_rocks() {
        use crate::glk::GlkBackend;
        let mut g = AppGlk::new(80, 24);
        let a = g.schannel_create(11);
        let b = g.schannel_create(22);
        assert_ne!(a, 0, "a created channel has a nonzero ref");
        assert_ne!(a, b, "distinct channels get distinct refs");
        assert_eq!(g.schannel_get_rock(a), 11);
        assert_eq!(g.schannel_get_rock(b), 22);
        assert_eq!(g.schannel_get_rock(9999), 0, "unknown channel → rock 0");
        // iterate: 0 → first, then next, then 0 at the end.
        let (first, first_rock) = g.schannel_iterate(0);
        assert_eq!((first, first_rock), (a, 11));
        let (second, second_rock) = g.schannel_iterate(first);
        assert_eq!((second, second_rock), (b, 22));
        assert_eq!(g.schannel_iterate(second), (0, 0), "past the end → (0,0)");
    }

    #[test]
    fn appglk_schannel_ops_buffer_in_order_with_volume_snapshot() {
        use crate::glk::GlkBackend;
        use crate::session::SchannelOp;
        let mut g = AppGlk::new(80, 24);
        let c = g.schannel_create(0);
        g.schannel_set_volume(c, 0x8000);          // half volume
        g.schannel_play(c, 5, 3, 9);               // play_ext(chan, snd, repeats, notify)
        g.schannel_stop(c);
        g.schannel_destroy(c);
        let ops = g.take_sound_ops();
        assert_eq!(ops, vec![
            SchannelOp::SetVolume { chan: c, vol: 0x8000 },
            SchannelOp::Play { chan: c, snd: 5, repeats: 3, notify: 9, volume: 0x8000 },
            SchannelOp::Stop { chan: c },
            SchannelOp::Destroy { chan: c },
        ]);
        assert!(g.take_sound_ops().is_empty(), "draining clears the buffer");
        assert_eq!(g.schannel_get_rock(c), 0, "destroy removed the channel");
    }

    #[test]
    fn appglk_play_snapshots_default_full_volume() {
        use crate::glk::GlkBackend;
        use crate::session::SchannelOp;
        let mut g = AppGlk::new(80, 24);
        let c = g.schannel_create(0); // no set_volume → default 0x10000 (Glk full)
        g.schannel_play(c, 1, 1, 0);
        let ops = g.take_sound_ops();
        assert_eq!(ops, vec![SchannelOp::Play { chan: c, snd: 1, repeats: 1, notify: 0, volume: 0x10000 }]);
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p app appglk_schannel`
Expected: FAIL — `no method named schannel_create`/`take_sound_ops` on `AppGlk`, `SchannelOp` unresolved.

- [ ] **Step 4: Add `AppGlk` channel state**

In `crates/app/src/glk_backend.rs`, add a small private struct above `pub struct AppGlk` (near ~96):

```rust
/// A live Glk sound channel's app-side state.
struct SoundChannel {
    rock: u32,
    /// Glk volume (0x10000 = full); snapshotted into each `Play` op.
    volume: u32,
}
```

Add fields to `struct AppGlk` (after `picts`, ~127):

```rust
    /// Live sound channels, keyed by Glk channel ref (BTree for stable iterate).
    schannels: BTreeMap<u32, SoundChannel>,
    /// Next channel ref to hand out (pre-incremented; first create → 1).
    next_schannel: u32,
    /// Buffered per-turn sound operations, drained by `take_sound_ops`.
    sound_ops: Vec<crate::session::SchannelOp>,
```

Initialize them in `with_graphics` (the struct literal, ~151, after `picts,`):

```rust
            schannels: BTreeMap::new(),
            next_schannel: 0,
            sound_ops: Vec::new(),
```

Add `take_sound_ops` in `impl AppGlk` (near `take_transcript_elems`, ~257):

```rust
    /// Drain the sound operations buffered this turn (see [`crate::session::SchannelOp`]).
    pub fn take_sound_ops(&mut self) -> Vec<crate::session::SchannelOp> {
        std::mem::take(&mut self.sound_ops)
    }
```

- [ ] **Step 5: Implement the `GlkBackend` schannel methods on `AppGlk`**

In `impl GlkBackend for AppGlk` (~544), add before `as_any` (~705):

```rust
    fn schannel_create(&mut self, rock: u32) -> u32 {
        self.next_schannel += 1;
        let id = self.next_schannel;
        self.schannels.insert(id, SoundChannel { rock, volume: 0x10000 });
        id
    }
    fn schannel_destroy(&mut self, chan: u32) {
        self.schannels.remove(&chan);
        self.sound_ops.push(crate::session::SchannelOp::Destroy { chan });
    }
    fn schannel_iterate(&mut self, chan: u32) -> (u32, u32) {
        let next = if chan == 0 {
            self.schannels.keys().next().copied()
        } else {
            self.schannels.range((chan + 1)..).next().map(|(k, _)| *k)
        };
        match next {
            Some(id) => (id, self.schannels.get(&id).map(|c| c.rock).unwrap_or(0)),
            None => (0, 0),
        }
    }
    fn schannel_get_rock(&mut self, chan: u32) -> u32 {
        self.schannels.get(&chan).map(|c| c.rock).unwrap_or(0)
    }
    fn schannel_play(&mut self, chan: u32, snd: u32, repeats: u32, notify: u32) -> u32 {
        let volume = self.schannels.get(&chan).map(|c| c.volume).unwrap_or(0x10000);
        self.sound_ops.push(crate::session::SchannelOp::Play { chan, snd, repeats, notify, volume });
        1
    }
    fn schannel_stop(&mut self, chan: u32) {
        self.sound_ops.push(crate::session::SchannelOp::Stop { chan });
    }
    fn schannel_set_volume(&mut self, chan: u32, vol: u32) {
        if let Some(c) = self.schannels.get_mut(&chan) {
            c.volume = vol;
        }
        self.sound_ops.push(crate::session::SchannelOp::SetVolume { chan, vol });
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p app appglk_schannel && cargo test -p app appglk_play_snapshots`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/session.rs crates/app/src/glk_backend.rs
git commit -m "$(cat <<'EOF'
feat(app): AppGlk sound-channel state + SchannelOp buffer

Quest: SQ-0208

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

### Task 6: app — `TurnResult.glulx_sound_ops` + `GlulxSession` wiring

**Files:**
- Modify: `crates/app/src/session.rs` — `TurnResult` gains `glulx_sound_ops: Vec<SchannelOp>`; update every `TurnResult { .. }` literal in the zvm path to add `glulx_sound_ops: Vec::new(),`
- Modify: `crates/app/src/glulx_session.rs` — `new` gains a `sound_enabled: bool` param + `set_sound` call; `finish_turn` drains sound ops; new `sound_notify` method
- Modify: `crates/app/src/main.rs` — thread `sound_enabled` into the 3 `GlulxSession::new` call sites (1688, 4123, 5245)

**Interfaces:**
- Consumes: `SchannelOp` (Task 5), `AppGlk::take_sound_ops` (Task 5), `Machine::set_sound` (Task 2), `Machine::deliver_sound_notify` (Task 4).
- Produces:
  - `TurnResult.glulx_sound_ops: Vec<SchannelOp>` (empty on the Z-machine path).
  - `GlulxSession::new(image, cols, rows, acceleration, graphics_enabled, sound_enabled, char_px, pict_blorb)` — new `sound_enabled: bool` param inserted **after** `graphics_enabled`.
  - `GlulxSession::sound_notify(&mut self, sound: u32, notify: u32) -> TurnResult`.

- [ ] **Step 1: Add the `TurnResult` field**

In `crates/app/src/session.rs`, in `pub struct TurnResult` (after `sounds`, ~186):

```rust
    /// Glk sound-channel operations emitted this turn (Glulx only; empty for the
    /// Z-machine, which uses `sounds`). Played by `AppState::play_glulx_sound_ops`.
    pub glulx_sound_ops: Vec<SchannelOp>,
```

The compiler will now flag every `TurnResult { .. }` literal as missing this field. Add `glulx_sound_ops: Vec::new(),` to each Z-machine-side literal (search: `rg 'TurnResult \{' crates/app/src`). Known sites include the zvm `collect_turn` in `session.rs` and any test fixtures; add the empty vec to all of them **except** the Glulx `finish_turn` literal (handled in Step 2).

- [ ] **Step 2: Drain sound ops in `GlulxSession::finish_turn`**

In `crates/app/src/glulx_session.rs` `finish_turn` (~199), just before the `TurnResult {` literal (after the `diagnostics`/`fault` lines, ~231), add:

```rust
        let glulx_sound_ops = self.appglk().take_sound_ops();
```

Then add the field to the returned literal (alongside `sounds: Vec::new(),`, ~239):

```rust
            glulx_sound_ops,
```

- [ ] **Step 3: Write the failing test for `new`'s signature + drain**

The `mod tests` block in `crates/app/src/glulx_session.rs` (~390) already has the helper `simple_line_image()` (a minimal Glulx image that prints a banner and waits for a line) and builds sessions via `GlulxSession::new(...)`. Add this test there. It targets the **new 8-arg signature** (with `sound_enabled` after `graphics_enabled`) and injects a buffered op through the `AppGlk` seam (a real game would call `glk_schannel_play`; the harness has no schannel-calling image, so it drives the backend directly):

```rust
    #[test]
    fn finish_turn_drains_buffered_sound_ops() {
        use crate::glk::GlkBackend;
        use crate::session::SchannelOp;
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, true, (1, 1), None)
            .expect("new");
        {
            let g = sess.appglk();
            let c = g.schannel_create(0);
            g.schannel_play(c, 7, 1, 0);
        }
        let result = sess.submit(""); // finish_turn drains the buffered op
        assert!(
            result.glulx_sound_ops.iter().any(|op| matches!(op, SchannelOp::Play { snd: 7, .. })),
            "buffered play reached the TurnResult: {:?}",
            result.glulx_sound_ops
        );
    }
```

(`appglk()` is a private method on `GlulxSession` — accessible from the in-module test.)

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test -p app finish_turn_drains_buffered_sound_ops`
Expected: FAIL — `GlulxSession::new` takes the wrong number of args / `glulx_sound_ops` missing.

- [ ] **Step 5: Add the `sound_enabled` param + `set_sound` + `sound_notify`**

In `crates/app/src/glulx_session.rs` `new` (~131), add the param after `graphics_enabled` and call `set_sound`:

```rust
    pub fn new(
        image: Vec<u8>,
        cols: u32,
        rows: u32,
        acceleration: bool,
        graphics_enabled: bool,
        sound_enabled: bool,
        char_px: (u32, u32),
        pict_blorb: Option<blorb::Blorb>,
    ) -> Result<GlulxSession, GError> {
```

After `machine.set_graphics(graphics_enabled);` (~145):

```rust
        machine.set_sound(sound_enabled);
```

Add the `sound_notify` method in `impl GlulxSession` (near `resize`, ~183):

```rust
    /// A sound finished: deliver a Glk `Evtype_SoundNotify` to the game and drive
    /// it to its next input request, returning the resulting turn (which carries
    /// any sound ops the game buffered while handling the notify — sound
    /// sequencing). A no-op turn once the game has quit.
    pub fn sound_notify(&mut self, sound: u32, notify: u32) -> TurnResult {
        if !self.quit {
            self.machine.deliver_sound_notify(sound, notify);
            let (pending, quit) = drive(&mut self.machine);
            self.pending = pending;
            self.quit = quit;
        }
        self.finish_turn()
    }
```

- [ ] **Step 6: Thread the arg into every `GlulxSession::new` call site**

Adding the param breaks every call. Update them all — insert the `sound_enabled` arg immediately after the `graphics_enabled` arg:

- `crates/app/src/main.rs` ~1688 (initial launch): add `cfg.enable_sound,` after the `cfg.images,` line.
- `crates/app/src/main.rs` ~4123 (restart/`reset_game`): add `state.config.enable_sound,` after the `state.config.images,` line.
- `crates/app/src/main.rs` ~5245 (test): `GlulxSession::new(bytes.clone(), 80, 24, true, false, (1, 1), None)` → insert `false` after the graphics `false`: `... true, false, false, (1, 1), None`.
- `crates/app/src/glulx_session.rs` `mod tests` (~390): ~12 existing call sites (lines ~543, 586, 663, 679, 716, 743, 767, 786, 806, 823, 834, 871). Insert `false` after the graphics bool in each — **except** the graphics test (~663) where graphics is `true`; it still gets `false` for sound: `...true, false, (2, 2)...` → `...true, false, false, (2, 2)...`. Let the compiler enumerate any missed site.

- [ ] **Step 7: Run the test + full app build to verify they pass**

Run: `cargo test -p app finish_turn_drains_buffered_sound_ops` then `cargo build -p app --tests`
Expected: PASS / clean build (all `TurnResult` literals and `GlulxSession::new` call sites updated).

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/session.rs crates/app/src/glulx_session.rs crates/app/src/main.rs
git commit -m "$(cat <<'EOF'
feat(app): thread Glulx sound through GlulxSession + TurnResult

Quest: SQ-0208

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

### Task 7: app — `AppState` playback + notify loop

**Files:**
- Modify: `crates/app/src/state.rs` — new fields `glulx_channels`, `glulx_sound_notify`; `play_glulx_sound_ops`; pure helpers `glk_volume_to_gain`, `glk_repeats_to_audio`; init the new maps where `sound_ids`/`sound_routines` are initialized
- Modify: `crates/app/src/main.rs` — `glulx_session_opt_mut` helper (~326); play glulx ops in `apply_turn_events` (~5064); extend the `finished()` poll (~2214)

**Interfaces:**
- Consumes: `SchannelOp` (Task 5), `TurnResult.glulx_sound_ops` + `GlulxSession::sound_notify` (Task 6), `AudioBackend::play_sample_gain`/`set_sample_gain` (Task 1).
- Produces:
  - `AppState::play_glulx_sound_ops(&mut self, ops: &[crate::session::SchannelOp])`
  - `pub fn glk_volume_to_gain(vol: u32) -> f32`
  - `fn glk_repeats_to_audio(repeats: u32) -> Option<u8>` (module-private)

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/app/src/state.rs`:

```rust
    #[test]
    fn glk_volume_to_gain_is_linear_over_0x10000() {
        assert_eq!(glk_volume_to_gain(0), 0.0);
        assert_eq!(glk_volume_to_gain(0x10000), 1.0);   // Glk full
        assert_eq!(glk_volume_to_gain(0x8000), 0.5);    // half
        assert!(glk_volume_to_gain(0x20000) > 1.0, "amplification passes through");
    }

    #[test]
    fn glk_repeats_to_audio_maps_counts_and_forever() {
        assert_eq!(glk_repeats_to_audio(0), None);            // play zero times → skip
        assert_eq!(glk_repeats_to_audio(1), Some(1));         // once
        assert_eq!(glk_repeats_to_audio(5), Some(5));         // N times
        assert_eq!(glk_repeats_to_audio(0xFFFF_FFFF), Some(255)); // -1 → forever
        assert_eq!(glk_repeats_to_audio(300), Some(254));     // clamp below the forever sentinel
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app glk_volume_to_gain glk_repeats_to_audio`
Expected: FAIL — functions not found.

- [ ] **Step 3: Add the pure helpers**

In `crates/app/src/state.rs`, near `sound_kind_to_format` (~255):

```rust
/// Map a Glk channel volume (`0x10000` = full; may exceed for amplification) to a
/// linear pre-master gain fraction for the audio backend.
pub fn glk_volume_to_gain(vol: u32) -> f32 {
    vol as f32 / 65536.0
}

/// Map Glk `repeats` to the audio backend's `repeats` byte, or `None` to skip
/// playing entirely. Glk: `0xFFFFFFFF` = loop forever; `0` = play zero times;
/// `N` = N plays. The audio byte reserves `255` for "forever", so finite counts
/// are clamped to `254`.
fn glk_repeats_to_audio(repeats: u32) -> Option<u8> {
    match repeats {
        0 => None,
        0xFFFF_FFFF => Some(255),
        n if n >= 255 => Some(254),
        n => Some(n as u8),
    }
}
```

- [ ] **Step 4: Add the `AppState` fields + initialization**

In `crates/app/src/state.rs`, add fields after `sound_routines` (~984):

```rust
    /// Playing Glulx sounds keyed by Glk channel ref (for stop / replace).
    pub glulx_channels: std::collections::HashMap<u32, audio::SoundId>,
    /// Pending sound-notify per playing SoundId: `(sound resource, notify value)`.
    pub glulx_sound_notify: std::collections::HashMap<audio::SoundId, (u32, u32)>,
```

Initialize both to `std::collections::HashMap::new()` wherever `sound_ids`/`sound_routines` are initialized (search: `rg 'sound_routines:' crates/app/src/state.rs` — add the two new fields to the same struct literal(s)).

- [ ] **Step 5: Add `play_glulx_sound_ops`**

In `crates/app/src/state.rs`, add a method to `impl AppState` (near `play_turn_sounds`, ~1386):

```rust
    /// Apply this turn's Glk sound-channel operations to the shared audio backend
    /// (Glulx). Mirrors `play_turn_sounds`: gated on the sound config flag and a
    /// present backend; resolves sound resources from `sound_blorb` and tracks the
    /// channel→SoundId and SoundId→notify maps.
    pub fn play_glulx_sound_ops(&mut self, ops: &[crate::session::SchannelOp]) {
        use crate::session::SchannelOp;
        if !self.config.enable_sound {
            return;
        }
        let Some(backend) = self.audio.as_mut() else { return };
        for op in ops {
            match *op {
                SchannelOp::Play { chan, snd, repeats, notify, volume } => {
                    // Playing on a busy channel stops the old sound first; the
                    // replaced sound fires no notify.
                    if let Some(old) = self.glulx_channels.remove(&chan) {
                        backend.stop(old);
                        self.glulx_sound_notify.remove(&old);
                    }
                    let Some(reps) = glk_repeats_to_audio(repeats) else { continue };
                    if let Some(blorb) = &self.sound_blorb {
                        if let Some((bytes, kind)) = blorb.sound(snd) {
                            if let Some(fmt) = sound_kind_to_format(kind) {
                                let gain = glk_volume_to_gain(volume);
                                if let Some(id) = backend.play_sample_gain(bytes, fmt, gain, reps) {
                                    self.glulx_channels.insert(chan, id);
                                    if notify != 0 {
                                        self.glulx_sound_notify.insert(id, (snd, notify));
                                    }
                                }
                            }
                        }
                    }
                }
                SchannelOp::Stop { chan } | SchannelOp::Destroy { chan } => {
                    if let Some(id) = self.glulx_channels.remove(&chan) {
                        backend.stop(id);
                        self.glulx_sound_notify.remove(&id);
                    }
                }
                SchannelOp::SetVolume { chan, vol } => {
                    if let Some(&id) = self.glulx_channels.get(&chan) {
                        backend.set_sample_gain(id, glk_volume_to_gain(vol));
                    }
                }
            }
        }
    }
```

- [ ] **Step 6: Run the helper tests to verify they pass**

Run: `cargo test -p app glk_volume_to_gain glk_repeats_to_audio`
Expected: PASS.

- [ ] **Step 7: Play glulx ops on every applied turn**

In `crates/app/src/main.rs`, in `apply_turn_events` (right after `state.play_turn_sounds(&result.sounds);`, ~5064):

```rust
    // Glulx Glk sound channels (empty for the Z-machine path).
    state.play_glulx_sound_ops(&result.glulx_sound_ops);
```

(`apply_turn_events` is called by both the normal submit path and `apply_game_driven_result`, so this covers ordinary turns and notify-driven turns.)

- [ ] **Step 8: Add `glulx_session_opt_mut` + extend the finished-sound poll**

In `crates/app/src/main.rs`, add near `zvm_session_opt_mut` (~326):

```rust
fn glulx_session_opt_mut(engine: &mut dyn Engine) -> Option<&mut GlulxSession> {
    engine.as_any_mut().downcast_mut::<GlulxSession>()
}
```

In the finished-sound poll loop (~2215-2231), after the existing Z-machine `sound_routines` block (after its closing braces, still inside `for id in done {`), add:

```rust
                // Glulx sound-notify: a finished channel delivers Evtype_SoundNotify.
                if let Some((snd, notify)) = state.glulx_sound_notify.remove(&id) {
                    state.glulx_channels.retain(|_, v| *v != id);
                    if let Some(gs) = glulx_session_opt_mut(&mut *session) {
                        let result = gs.sound_notify(snd, notify);
                        if apply_game_driven_result(
                            &mut state, &mut mapper, &result, &save_dir, &ifid, last_panes.map,
                        ) {
                            break 'event_loop;
                        }
                    }
                }
```

- [ ] **Step 9: Build + test the app**

Run: `cargo build -p app && cargo test -p app`
Expected: clean build, all tests green.

- [ ] **Step 10: Commit**

```bash
git add crates/app/src/state.rs crates/app/src/main.rs
git commit -m "$(cat <<'EOF'
feat(app): play Glulx schannel ops + deliver sound-notify events

Quest: SQ-0208

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

### Task 8: README feature note + full-workspace verification

**Files:**
- Modify: `README.md` — add a Glulx sound bullet to the feature overview

**Interfaces:** none (docs + final verification).

- [ ] **Step 1: Add the README note**

In `README.md`, find the feature list that already mentions Z-machine sound and/or in-game graphics. Add a concise bullet in the same style, e.g.:

```markdown
- **Glulx sound** — Glk sound channels (`glk_schannel_*`) play a blorb's AIFF/OGG/MOD resources, with per-channel volume and sound-finished (notify) events. Complements the existing Z-machine `@sound_effect` support.
```

Match the surrounding bullet style/tense exactly; do not restructure the section.

- [ ] **Step 2: Full workspace build + test**

Run: `cargo build && cargo test`
Expected: entire workspace builds; all tests pass.

- [ ] **Step 3: Manual smoke check (record the result)**

Run the app on the reference story and confirm sound plays:

```bash
cargo run -p app -- stories/sensory.blorb
```

At the prompt, type `hit gong`. Expected: the gong sound plays and the "[Your interpreter does not support sound...]" message no longer appears. (If run in a headless/CI environment with no audio device, note that the message is gone — gestalt now reports support — even if no device is available to actually sound it.)

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "$(cat <<'EOF'
docs(readme): note Glulx/Glk sound support

Quest: SQ-0208
Completes: SQ-0208

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

## Notes for the executor

- **Zero-dep guard:** after Tasks 2–4, confirm `gvm` still has no new dependencies (`crates/gvm/Cargo.toml` unchanged). All audio/`SchannelOp` types stay in `app`/`audio`.
- **`TurnResult` literal churn (Task 6):** adding a struct field breaks every literal; let the compiler enumerate them and add `glulx_sound_ops: Vec::new(),` to each Z-machine-side one. This is the one place the change fans out.
- **Borrow pattern (Task 7):** `play_glulx_sound_ops` binds `backend = self.audio.as_mut()` and then reads other `self` fields directly — the same disjoint-field-borrow shape `play_turn_sounds` already uses; keep field accesses direct (not via a method) so the borrow checker allows it.
- **Device-free CI:** `play_sample_gain` returns `None` with no audio device, so `glulx_channels` stays empty in unit tests — the behavioral coverage is the pure helpers plus the gvm dispatch/notify tests; do not write playback tests that assume a device.
