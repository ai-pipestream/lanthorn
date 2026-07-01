# Sound Support — Sub-project A: Cross-platform Audio Backend + Z-Machine Sound

**Date:** 2026-07-01
**Scope:** A cross-platform host-side audio backend and Z-machine sound
(`sound_effect`, VAR:0x15): real tones for the built-in bleeps (#1/#2) and
sampled playback of Blorb `Snd ` resources (#≥3), in the app and `zvm-cli`.
**Out of scope (Sub-project B, separate spec):** Glulx/Glk sound channels
(`glk_schannel_*`) — they will reuse this backend.
**Validation target:** The Lurking Horror (`stories/lurkinghorror-r219-s870912.z3`
+ sibling `stories/Lurking.blb`) plays its sampled sounds in both hosts.

> **STATUS: REVISED 2026-07-01, ready for review.** Timed input has landed
> (`run_timed_interrupt` + the host input-loop interleave). §7 is now written
> properly against that machinery: the sound finish-routine callback is
> **included** in this sub-project (§7.1) rather than deferred, since the
> re-entrant routine execution and the host tick both already exist. MOD
> (tracker) playback is also **included** now, via the pure-`std` MIT
> `mod_player` crate streamed through `rodio` (§3), behind a default-on
> `mod-music` feature. Only `prepare` preloading remains deferred (§7.2).

## 1. Background

`sound_effect number effect volume routine` (ZMSD §9.4): `number` 1/2 are the
built-in high/low "bleeps"; `number ≥ 3` selects a sampled sound from the
story's Blorb `Snd ` resources. `effect` is 1=prepare, 2=start, 3=stop,
4=finish. `routine` (v5+) is called when the sound finishes.

Today (`crates/zvm/src/cpu/exec.rs:1113`) the engine records `Beep::High`/`Low`
into `pending_beeps` for #1/#2 (hosts render a border pulse — the 2026-06-25
sound-event-visualization design) and logs a diagnostic for #≥3. No audio is
produced anywhere; no audio dependency exists in the workspace.

**Constraints:** the VM crates (`zvm`, `gvm`) stay **zero-dependency** — they
only record sound events. All audio lives in the hosts. Everything must build
and run on macOS, Windows, and Linux.

## 2. Architecture

Three layers:
1. **`crates/audio`** (new, host-side): a `rodio`-backed backend that plays
   generated tones and decoded samples. Engine-agnostic — Sub-project B reuses
   it.
2. **`crates/blorb`** (existing): gains an accessor to fetch a `Snd ` resource's
   payload bytes + detected format.
3. **`crates/zvm`** (existing, zero-dep): records a richer `SoundEvent` the
   hosts drain and act on.

The hosts (`app`, `zvm-cli`) own an `AudioBackend` and the loaded Blorb sound
map, drain the engine's sound events each turn, and play tones / samples.

## 3. `crates/audio` — the backend

New workspace crate depending on `rodio` and (behind the `mod-music` feature)
`mod_player`.

- **Feature gate:** `default = ["playback", "mod-music"]`.
  - `playback` off → the crate compiles to no-ops (headless/CI builds need no
    ALSA / audio device). At runtime, if no output device is available, the
    backend degrades to silent (logs once; never panics or blocks).
  - `mod-music` off → the `mod_player` dependency and the MOD decode path drop
    out; `SoundFormat::Mod` then logs "unsupported sound format" and plays
    nothing (same as an unknown format). Gated separately so the unmaintained
    (2019, MIT, pure-`std`) `mod_player` can be dropped without losing AIFF/Ogg.
- **API** (stable across both engines):
  ```rust
  pub struct AudioBackend { /* Option<OutputStream> + mixer/sinks */ }
  pub enum SoundFormat { Aiff, Ogg, Mod }      // Mod behind `mod-music` feature
  pub type SoundId = u32;

  impl AudioBackend {
      pub fn new(volume: u8) -> AudioBackend;   // silent if unavailable/disabled
      pub fn play_tone(&mut self, freq_hz: f32, ms: u32, volume: u8);
      pub fn play_sample(&mut self, bytes: &[u8], format: SoundFormat,
                         volume: u8, repeats: u8) -> Option<SoundId>;
      pub fn stop(&mut self, id: SoundId);
      pub fn stop_all(&mut self);
      pub fn set_volume(&mut self, volume: u8);  // 0..=100
  }
  ```
- **Bleeps:** `play_tone` synthesises a short PCM buffer (a decaying sine, ~800
  Hz for high / ~400 Hz for low, ~150 ms) and plays it via a `rodio::Sink`.
- **Sampled playback:** `play_sample` decodes the payload to PCM and plays it
  non-blocking on its own `Sink`. `repeats` maps the Z-machine repeat count
  (255 = loop forever; 0/omitted = play once; 1..=254 = that many plays —
  interpreter parity, per the host's `repeat_plan`). `SoundId` lets `stop` target a playing
  sound (needed for `effect` 3 = stop).
- **AIFF decode (in-crate):** parse the IFF `FORM`/`AIFF` container — `COMM`
  chunk (channels, sample rate, bit depth; 80-bit IEEE extended sample-rate
  field) and `SSND` chunk (PCM, big-endian) — into interleaved `i16` samples
  fed to `rodio::buffer::SamplesBuffer`. AIFF is what Infocom-era Blorb sounds
  use (Lurking Horror, Sherlock).
- **Ogg decode:** via rodio's `Decoder` (vorbis feature).
- **MOD playback (`mod-music` feature):** `mod_player::read_mod_file_slice(bytes)
  -> Song` parses the in-memory Blorb `MOD ` payload (no temp file); a small
  `ModSource` adapter implements `rodio::Source` (channels = 2, sample rate =
  the output stream's rate) by pulling `mod_player::next_sample(&song, &mut
  state) -> (f32, f32)` per frame and yielding interleaved `f32`. Playback is a
  streaming `Sink` like any other sound, so `stop`/`stop_all` and the `finished`
  query (§7.1) work unchanged. `mod_player` is pure-`std` (its cpal/hound/serde
  deps are dev-only), so it adds no platform dependency. Handles Amiga
  ProTracker `.mod` (what Blorb `MOD ` resources are), not XM/S3M/IT.
- Non-blocking throughout: playback runs on rodio's audio thread; host code
  never blocks on sound.

## 4. `crates/blorb` — Snd accessor

The resource index already carries `Snd ` entries with `chunk_type` + `len`.
Add:
```rust
impl Blorb {
    /// Payload bytes + detected format for sound resource `number`, if present.
    pub fn sound(&self, number: u16) -> Option<(&[u8], SoundKind)>;
}
pub enum SoundKind { Aiff, Ogg, Mod, Other }   // from the chunk type
```
Format is detected from the resource's chunk type (`FORM` → AIFF, `OGGV` → Ogg,
`MOD ` → Mod). `Aiff`/`Ogg`/`Mod` are played by the backend (`Mod` only when the
`mod-music` feature is on). `Other` — and `Mod` with the feature off — is
surfaced but unsupported (host logs "unsupported sound format", plays nothing).

**Host sound-resource loading:** the host resolves the sound container once at
launch: if the loaded story is itself a Blorb, use its `Snd ` resources;
otherwise look for a **sibling** `.blb`/`.blorb` next to the story file (mirror
the hint-file sibling discovery in `hints.rs`). Lurking Horror is
`lurkinghorror-*.z3` + `Lurking.blb`.

## 5. `crates/zvm` — SoundEvent model (zero-dep)

Generalise the bleep recording. Replace/augment `pending_beeps` with:
```rust
pub struct SoundEvent {
    pub number: u16,   // 1/2 = bleep; >=3 = Blorb Snd resource
    pub effect: u8,    // 1=prepare 2=start 3=stop 4=finish
    pub volume: u8,    // 1..=8 (Z-scale) or 255 = loudest
    pub repeats: u8,   // repeat count (255 = forever; 0/omitted = play once)
    pub routine: u16,  // finish-routine; the host calls it on sound end (see §7.1)
}
pub pending_sounds: Vec<SoundEvent>,   // drained by the host
```
`sound_effect` (exec.rs:1113) records a `SoundEvent` for every call (including
#1/#2, so bleeps carry volume too). The engine remains zero-dep — it only
records. The border-pulse visualisation reads the same events for #1/#2 (and
optionally #≥3), so the visual cue is preserved (see §6).

*(The existing `Beep`/`pending_beeps` API and its tests migrate to
`SoundEvent`/`pending_sounds`; the border-pulse host code updates to read the
new field. This is a small, contained refactor within this sub-project.)*

## 6. Host wiring (app + zvm-cli)

- Each host constructs an `AudioBackend` at startup (from `enable_sound` +
  `volume`) and resolves the Blorb sound map (§4).
- After each turn (where the host already drains beeps for the border pulse),
  process `pending_sounds`:
  - `number` 1/2 → `play_tone` (high/low) at the event volume.
  - `number` ≥ 3, `effect` 2 (start) → look up the Blorb resource, `play_sample`
    with volume + repeats; remember its `SoundId` keyed by `number`.
  - `effect` 3 (stop) → `stop` the `SoundId` for that `number`.
  - `effect` 1 (prepare) → no-op (an optimisation; we decode on start).
  - The finish-routine is not fired from a `sound_effect` event; the host fires
    it when the sound actually ends, via the completion poll in §7.1.
- **Config / flags** (mirrors the timed-input surface):
  - `app`: `enable_sound: bool` (default `true`) + `volume: u8` (default 100) in
    `config.rs`; slash commands `toggle-sound` and `volume <0-100>`; a settings
    (F2) row. Follow the `honor_game_colours` plumbing pattern.
  - `zvm-cli`: `--no-sound` and `--volume <0-100>` flags.
- **Border-pulse visual cue retained** as a complementary/accessibility signal
  (and the only cue when sound is disabled).

## 7. Finish-routine callback + remaining deferrals

### 7.1 Finish-routine callback (INCLUDED — reuses the timed-input machinery)

`sound_effect`'s `routine` operand (v5+) is a routine the game wants called when
the sound finishes. Timed input already built both halves this needs:

- **Engine — generalize the routine runner.** `run_timed_interrupt` (shipped)
  already pushes a routine frame, steps it to completion, and captures its return
  via the eval stack, with a nested-input guard that snapshots/restores
  `pending_input`. Extract that core into a general
  `pub fn run_routine(&mut self, packed_routine: u16) -> u16` (returns the
  routine's value) and have `run_timed_interrupt` call it (`aborted = ret != 0`).
  `run_routine` is safe to call whether or not a read is pending — it doesn't
  disturb `pending_input` on the normal path and restores it on the nested bail.
  Zero new engine deps.
- **Host — detect sound end at the existing input tick.** When the host starts a
  sample with a nonzero finish-routine (`effect` 2), it remembers
  `(SoundId, routine)`. The input loop already polls on a deadline for timed
  input; at that same point each host also asks the `AudioBackend` which
  `SoundId`s have finished (a `rodio::Sink::empty()` check per tracked sound).
  For each finished sound with a routine, call `engine.run_routine(routine)` and
  re-render (the routine may print). The finish-routine's return value is
  ignored (unlike timed input, it does not abort anything — ZMSD §9.4).
- **Backend addition:** `AudioBackend::finished(&self) -> Vec<SoundId>` (or
  `is_finished(id)`) so the host can drain completions. No blocking.

This is a small increment on top of §3–§6 now that the interrupt plumbing
exists; it makes sampled sounds that chain/trigger game events (via their finish
routine) work rather than silently dropping the callback.

### 7.2 Still deferred

- **`prepare` (effect 1) preloading:** treated as a no-op (we decode on start).

## 8. Testing / validation

- **`crates/audio` unit tests:** AIFF parse (COMM fields + SSND PCM length/first
  samples on a tiny hand-built AIFF); tone PCM generation (length + non-silence).
  MOD (`mod-music` feature): a tiny checked-in `.mod` fixture decodes via
  `read_mod_file_slice` and the `ModSource` reports 2 channels and yields
  non-silent interleaved `f32` for the first N frames. Playback itself is
  validated manually (no device in CI); the no-op / no-device path — and the
  MOD path with `mod-music` off — is unit-tested to never panic.
- **`crates/blorb` test:** `sound(n)` returns the right payload + kind for a
  small synthetic Blorb with a `Snd ` (AIFF) resource.
- **`crates/zvm` test:** `sound_effect` records a `SoundEvent` with the correct
  number/effect/volume/repeats for #1/#2 and #≥3 (migrated from the existing
  bleep tests). `run_routine` runs a routine to completion and returns its value
  (mirroring the existing `run_timed_interrupt` tests, which now delegate to it).
- **Manual (both hosts):** The Lurking Horror — trigger a scene with sound
  (e.g. the elevator / footsteps), confirm audio plays; `--no-sound` / app
  `toggle-sound` silences it; `--volume` / `/volume` scales it; the border pulse
  still fires.

## 9. Cross-platform notes

- `rodio` reaches CoreAudio (macOS), WASAPI (Windows), ALSA (Linux). On Linux,
  building with the `playback` feature requires the ALSA dev headers
  (`libasound2-dev` / `alsa-lib-devel`). Document this in the README build
  section.
- The `playback` feature (default on) can be disabled for constrained/headless
  builds; the backend is then a compile-time no-op.
- `mod_player` (the `mod-music` feature, default on) is pure-`std` and adds no
  platform-specific build requirement beyond what `rodio` already needs.
- Runtime: absence of an output device degrades to silent (logged once).

## 10. File / crate map

| Crate/file | Change |
|-----------|--------|
| `crates/audio/` (new) | `rodio` backend, tone synthesis, AIFF decode, MOD playback via `mod_player` + `ModSource` (behind `mod-music`), `SoundFormat`, feature gates, `finished()` completion query (§7.1) |
| `crates/blorb/src/lib.rs` | `sound(number)` accessor + `SoundKind` |
| `crates/zvm/src/cpu/exec.rs` | `SoundEvent` + `pending_sounds` (replaces `pending_beeps`); `sound_effect` records events; extract `run_routine` from `run_timed_interrupt` for the finish callback (§7.1); zero-dep |
| `crates/zvm-cli/src/main.rs` | own `AudioBackend` + Blorb sound map; drain events; poll sound completions → `run_routine`; `--no-sound` / `--volume` |
| `crates/app/src/*` | own `AudioBackend`; drain events; poll sound completions → `run_routine` at the input tick; `enable_sound`/`volume` config + `toggle-sound`/`volume` slash + settings row; update border-pulse to read `pending_sounds` |
| `README.md` | audio feature + Linux ALSA build prereq |
