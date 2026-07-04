# Glk/Glulx Sound in gvm — Design

**Date:** 2026-07-04
**Status:** Approved (design), pending spec review
**Quest:** SQ-0208
**Reference story:** `stories/sensory.blorb` (Glulx; carries AIFF `Snd ` resources)

## Goal

Play Glk sound channels for Glulx games. Games call `glk_schannel_*`
functions to create channels, play `Snd ` resources from the blorb, stop them,
and set volume; sound-finished (`Evtype_SoundNotify`) events are delivered back
to the VM so games can sequence sounds. Today gvm implements **no** Glk sound:
`glk_gestalt(Sound)` returns 0 via the catch-all, the schannel selectors hit
the "unhandled @glk selector" diagnostic, and `GlulxSession` hardcodes
`sounds: Vec::new()`.

This mirrors the already-shipped Z-machine sound path (`@sound_effect` →
`SoundEvent` → `AppState::play_turn_sounds` → the `audio` crate) and reuses the
entire `audio` crate and blorb sound-resolution path unchanged.

## Decisions (locked)

| Question | Decision |
|----------|----------|
| Selector scope | **Base Sound only.** `0x00F0–0x00FC`: create, destroy, iterate, get_rock, play, play_ext, stop, set_volume, load_hint. No Sound2 (pause/unpause/create_ext/play_multi/set_volume_ext). |
| Notify events | **Implemented.** `glk_schannel_play_ext` carries a notify value; a finished sound delivers `Evtype_SoundNotify` back into the VM. `gestalt_SoundNotify` reports 1. |
| Playback ownership | **Single shared `AudioBackend`.** Channel *state* lives in `AppGlk`; *playback* runs through `AppState.audio` so the existing volume/mute UI governs Glulx sound. |
| Per-channel volume | **Honored** via two small additive `audio`-crate methods (linear gain). |
| `schannel_play` return | Returns **1 optimistically** (does not verify the resource exists). Same known limitation as `graphics_draw_image` (SQ-0175); a follow-up, not solved here. |

## Non-goals (this pass)

- Sound2 gestalt (21) and its selectors (`0x00F4–0x00F7`, `0x00FD`):
  `schannel_create_ext`, `schannel_play_multi`, `schannel_pause`,
  `schannel_unpause`, `schannel_set_volume_ext` (volume fades with duration).
  `gestalt_Sound2` reports 0.
- Verifying a sound resource exists at `schannel_play` time (optimistic 1).
- `glk_sound_load_hint` prefetch/decoding (accepted as a no-op).

## Background: the existing seams

Reconnaissance established the exact models this design leans on.

### Z-machine sound pipeline (the template)

```
zvm (zero-dep)      app/session.rs         app/state.rs            audio crate         blorb crate
@sound_effect  ─► SoundEvent ─► TurnResult.sounds ─► play_turn_sounds ─► AudioBackend ◄─ Blorb::sound(n)
```

- `AudioBackend` (`crates/audio/src/lib.rs:214`) is feature-gated (`playback`,
  default on; no-op struct at `:353` otherwise). Key methods:
  `play_sample(bytes, format, z_volume, repeats) -> Option<SoundId>` (`:260`),
  `stop(id)` (`:308`), `set_volume(master)` (`:323`),
  `finished() -> Vec<SoundId>` (`:334`). It takes **raw encoded resource
  bytes** and decodes internally (AIFF/OGG/MOD). `SoundId = u32` (`:6`),
  `SoundFormat { Aiff, Ogg, Mod }` (`:10`).
- `Blorb::sound(n) -> Option<(&[u8], SoundKind)>` (`crates/blorb/src/lib.rs:186`)
  fetches a `Snd ` resource by number; `SoundKind { Aiff, Ogg, Mod, Other }`.
  `resolve_sound_blorb` (`:233`) runs for **every** game, so
  `AppState.sound_blorb` (`crates/app/src/state.rs:980`) is already populated
  for Glulx games. `AppState.audio` (`state.rs:978`) already exists.
- `AppState::play_turn_sounds` (`state.rs:1348`) drives playback and tracks
  `sound_ids`/`sound_routines`. The idle loop polls `audio.finished()`
  (`main.rs:2214`) and fires the v5 finish routine for the Z-machine.
- `sound_kind_to_format(SoundKind) -> Option<SoundFormat>` (`state.rs:255`).

### gvm Glk dispatch + event model

- `Machine::glk_dispatch(&mut self, selector, args) -> R<u32>`
  (`crates/gvm/src/exec.rs:2315`). Args via `let a = |i| args.get(i)...`.
  Graphics arms (the pattern to copy) at `0x00E0–0x00EB`, each gated on
  `if self.graphics_enabled`. Catch-all `other =>` pushes the "unhandled @glk
  selector" diagnostic and returns 0 (`exec.rs:2766`). Sound arms
  (`0x00F0–0x00FC`) insert immediately before that catch-all.
- `graphics_enabled: bool` (`exec.rs:143`), `set_graphics(on)` (`exec.rs:1725`),
  `glk_gestalt(sel, val)` (`exec.rs:3128`).
- `GlkBackend` trait defined in gvm (`crates/gvm/src/glk.rs:270`), implemented
  by `AppGlk` in the app (`crates/app/src/glk_backend.rs:544`). gvm holds it as
  `Box<dyn GlkBackend>` (`exec.rs:125`). Zero-dep seam: primitive args only.
- **Events:** `GlkEvent { etype, win, val1, val2 }` (`glk.rs:232`);
  `evtype::SOUND_NOTIFY = 7` already declared (`glk.rs:173`). `glk_select`
  (`exec.rs:2938`) delivers a queued non-input event first, else suspends into
  `pending_input` and returns `StepResult::NeedLine/NeedChar` to the host.
  `write_event(addr, ev)` (`exec.rs:2985`) writes via the standard out-ref
  convention. Async events use two paths, both present for Arrange:
  - queued (not blocked): `push_event` (`glk.rs:1319`), drained on next select;
  - direct (blocked): `deliver_arrange` (`exec.rs:3101`) writes into the
    suspended select and **takes** `pending_input` without consuming the
    window's line/char request, so the game loops back and re-suspends.
- `GlulxSession` (`crates/app/src/glulx_session.rs`): `new(...)` (`:131`) builds
  `AppGlk` and calls `machine.set_graphics(...)`; `drive()` (`:100`) steps to
  the next input; `resize()` (`:173`) = `machine.rearrange()` + re-drive (the
  notify injector's template); `finish_turn()` (`:199`) builds `TurnResult`
  and hardcodes `sounds: Vec::new()` (`:239`). `TurnResult` at
  `crates/app/src/session.rs:169`.

## The design

### 1. gvm — capability + state (zero-dep)

Add, mirroring graphics exactly:

```rust
// crates/gvm/src/exec.rs
pub(crate) sound_enabled: bool,          // near graphics_enabled (:143), default false

pub fn set_sound(&mut self, on: bool) {  // near set_graphics (:1725)
    self.sound_enabled = on;
}
```

`glk_gestalt` (`exec.rs:3128`) gains, gated on `sound_enabled`:

```rust
8  => self.sound_enabled as u32,   // gestalt_Sound
9  => self.sound_enabled as u32,   // gestalt_SoundVolume
10 => self.sound_enabled as u32,   // gestalt_SoundNotify
// 21 (gestalt_Sound2) stays 0 via the catch-all
```

### 2. gvm — schannel dispatch arms

Inserted before the catch-all at `exec.rs:2766`, each gated
`if self.sound_enabled { ... } else { 0 }`:

| Sel | Function | Backend call | Returns |
|-----|----------|--------------|---------|
| 0x00F0 | schannel_create | `backend.schannel_create(a(0))` | chan ref |
| 0x00F1 | schannel_destroy | `backend.schannel_destroy(a(0))` | 0 |
| 0x00F2 | schannel_iterate | `(next, rock) = backend.schannel_iterate(a(0))`; write `rock` to the out-ref `a(1)` via `glk_out_ref` | next ref |
| 0x00F3 | schannel_get_rock | `backend.schannel_get_rock(a(0))` | rock |
| 0x00F8 | schannel_play | `backend.schannel_play(a(0), a(1), 1, 0)` | 1/0 |
| 0x00F9 | schannel_play_ext | `backend.schannel_play(a(0), a(1), a(2), a(3))` | 1/0 |
| 0x00FA | schannel_stop | `backend.schannel_stop(a(0))` | 0 |
| 0x00FB | schannel_set_volume | `backend.schannel_set_volume(a(0), a(1))` | 0 |
| 0x00FC | sound_load_hint | no-op (like `window_flow_break`) | 0 |

New `GlkBackend` methods (no-op defaults, primitive args only), added near the
graphics methods in `crates/gvm/src/glk.rs:320`:

```rust
fn schannel_create(&mut self, _rock: u32) -> u32 { 0 }
fn schannel_destroy(&mut self, _chan: u32) {}
fn schannel_iterate(&mut self, _chan: u32) -> (u32, u32) { (0, 0) } // (next_ref, rock)
fn schannel_get_rock(&mut self, _chan: u32) -> u32 { 0 }
fn schannel_play(&mut self, _chan: u32, _snd: u32, _repeats: u32, _notify: u32) -> u32 { 0 }
fn schannel_stop(&mut self, _chan: u32) {}
fn schannel_set_volume(&mut self, _chan: u32, _vol: u32) {}
```

`schannel_iterate`: gvm calls the backend for `(next, rock)`, then writes
`rock` to the caller's out-ref pointer `a(1)` using the existing `glk_out_ref`
convention (ptr 0 discards, 0xFFFFFFFF pushes to stack, else writes at ptr).

### 3. gvm — notify injection

Mirror `deliver_arrange` (`exec.rs:3101`):

```rust
// crates/gvm/src/exec.rs
pub fn deliver_sound_notify(&mut self, sound: u32, notify: u32) {
    let ev = GlkEvent { etype: evtype::SOUND_NOTIFY, win: 0, val1: sound, val2: notify };
    if let Some(pi) = self.pending_input.take() {
        let _ = self.write_event(pi.event_addr, ev);   // blocked at select → direct write
    } else {
        self.glk.push_event(ev);                        // not blocked → queue for next select
    }
}
```

Reuses `GlkEvent`, `evtype::SOUND_NOTIFY`, `write_event`, `pending_input`,
`push_event` — all already present. The `push_event` dedupe guard only matches
Arrange/Redraw, so distinct sound-notifies never collapse. Direct-write path
takes `pending_input` but leaves the window's line/char request intact (as
Arrange does), so the game handles the notify and re-suspends on the still
pending read.

### 4. app — `AppGlk` channel state (app crate)

`AppGlk` owns the channel model and an ordered op buffer. `SchannelOp` is an
**app-crate** enum — gvm never sees it.

```rust
// crates/app/src/glk_backend.rs (or a small sibling module)
pub enum SchannelOp {
    // `volume` is the channel's current volume snapshotted at buffer time, so
    // AppState (which cannot see AppGlk's channel table) can compute gain.
    Play { chan: u32, snd: u32, repeats: u32, notify: u32, volume: u32 },
    Stop { chan: u32 },
    SetVolume { chan: u32, vol: u32 },
    Destroy { chan: u32 },
}

struct SoundChannel { rock: u32, volume: u32 } // volume default 0x10000 (Glk full)
// on AppGlk:
schannels: BTreeMap<u32, SoundChannel>,   // ref → channel (BTree for stable iterate order)
next_schannel: u32,                        // ref counter, starts at 1
sound_ops: Vec<SchannelOp>,                // buffered per-turn, drained by finish_turn
```

`GlkBackend for AppGlk` implementations:

- `schannel_create(rock)`: `next_schannel += 1`-style allocation of a fresh
  nonzero ref; insert `SoundChannel { rock, volume: 0x10000 }`; return ref.
- `schannel_destroy(chan)`: remove from `schannels`; push `Stop { chan }` then
  drop the channel. (Buffer `Destroy`/`Stop` so playback stops the live sound.)
- `schannel_iterate(chan)`: `chan == 0` → first entry; else the entry after
  `chan`. Return `(next_ref_or_0, that_channel_rock_or_0)`.
- `schannel_get_rock(chan)`: stored rock or 0.
- `schannel_play(chan, snd, repeats, notify)`: push
  `Play { chan, snd, repeats, notify, volume: schannels[chan].volume }`; return
  1. (Unknown `chan` → volume defaults to `0x10000`.)
- `schannel_stop(chan)`: push `Stop { chan }`.
- `schannel_set_volume(chan, vol)`: update `schannels[chan].volume`; push
  `SetVolume { chan, vol }`.

A new backend drain method (mirroring `take_transcript_elems`):
`take_sound_ops(&mut self) -> Vec<SchannelOp>` returns and clears `sound_ops`.

### 5. app — session plumbing

- `GlulxSession::new` gains a `sound_enabled: bool` param (after
  `graphics_enabled`), and calls `machine.set_sound(sound_enabled)`. Threaded
  at the 3 call sites (`main.rs:1688`, `:4123`, `:5245`) from the sound config
  flag (`config.enable_sound`).
- `TurnResult` (`session.rs:169`) gains `glulx_sound_ops: Vec<SchannelOp>`
  (default empty; zvm leaves it empty).
- `GlulxSession::finish_turn` (`:199`) sets
  `glulx_sound_ops: self.appglk().take_sound_ops()` (leaving the existing
  `sounds: Vec::new()` for the Z-machine field).
- New `GlulxSession::sound_notify(&mut self, sound: u32, notify: u32) -> TurnResult`
  mirroring `resize`: guard on quit; `self.machine.deliver_sound_notify(...)`;
  `drive`; `finish_turn()`. The returned `TurnResult` carries any sound ops the
  game buffered while handling the notify (sound sequencing).

### 6. app — playback (`AppState`)

```rust
// crates/app/src/state.rs
glulx_channels: HashMap<u32, audio::SoundId>,           // chan ref → live sound
glulx_sound_notify: HashMap<audio::SoundId, (u32, u32)>, // id → (snd, notify)

pub fn play_glulx_sound_ops(&mut self, ops: &[SchannelOp]) { ... }
```

Gated on the sound config flag + `self.audio.is_some()`. Per op:

- `Play { chan, snd, repeats, notify, volume }`: if `glulx_channels` already has
  `chan`, `stop` that `SoundId` first and drop it from `glulx_sound_notify`
  (replacing a playing sound fires **no** notify for the old one). Resolve bytes
  via `sound_blorb.sound(snd)` → `sound_kind_to_format`. Compute gain from the
  op's `volume` (§7). Map Glk `repeats` (0xFFFFFFFF = forever → the
  audio crate's forever; N → N plays; 0 → skip). Call
  `audio.play_sample_gain(bytes, fmt, gain, repeats_u8) -> Option<SoundId>`; on
  `Some(id)` set `glulx_channels[chan] = id` and, if `notify != 0`,
  `glulx_sound_notify[id] = (snd, notify)`.
- `Stop { chan }` / `Destroy { chan }`: `glulx_channels.remove(chan)` → `stop`
  the id and drop it from `glulx_sound_notify` (stopping fires no notify).
- `SetVolume { chan, vol }`: update any live sound via
  `audio.set_sample_gain(id, gain_from(vol))`.

`AppState` cannot see `AppGlk`'s channel table, so `SchannelOp::Play` carries
the channel's current `volume` snapshotted at buffer time (§4).

### 7. audio crate — linear gain (additive)

Glk volume is linear on `0..0x10000`; the crate's `z_volume: u8` path treats
0 as "full", so it cannot represent Glk volume. Add two methods next to the
existing ones (Z-machine path untouched):

```rust
// crates/audio/src/lib.rs
pub fn play_sample_gain(&mut self, bytes: &[u8], format: SoundFormat,
                        gain: f32, repeats: u8) -> Option<SoundId>;
pub fn set_sample_gain(&mut self, id: SoundId, gain: f32);
```

`gain` is the pre-master fraction; final Sink volume = `master/100 * gain`,
matching how `play_sample` combines master with its per-sound level. The no-op
backend (`lib.rs:353`) gets matching stubs. `AppState` computes
`gain = (glk_vol as f32 / 65536.0)` (clamped `>= 0.0`; Glk allows > 1.0 for
amplification, passed through).

### 8. Notify loop (extends `main.rs:2214`)

The idle loop already polls `audio.finished()` and, per finished `SoundId`,
fires the Z-machine finish routine. Extend it: for a Glulx session, look up the
id in `glulx_sound_notify`; on a hit, remove it, drop the id from
`glulx_channels`, call `session.sound_notify(snd, notify)`, and feed the
returned `TurnResult` through `apply_game_driven_result` **and**
`play_glulx_sound_ops(&result.glulx_sound_ops)` — so a game that starts the
next sound on a notify keeps its sequence playing. The normal `submit` path
also calls `play_glulx_sound_ops(&result.glulx_sound_ops)` alongside the
existing `play_turn_sounds`.

### 9. Fallback

When the sound config flag is off or `AppState.audio` is `None`,
`play_glulx_sound_ops` is a no-op and gvm's `sound_enabled` is false — gestalt
reports 0 and the schannel arms return 0, so a spec-correct game reports "no
sound" and never calls the channels. Consistent with the graphics fallback.

### 10. Audit correction

`docs/superpowers/audits/2026-06-30-glulx-feature-gaps.md:302,310` has the sound
and hyperlink selector ranges swapped. Correct to: sound `0x00F0–0x00FC`
(Sound2 adds `0x00F4–0x00F7`, `0x00FD`), hyperlinks `0x0100–0x0103`.

## Testing

**gvm (zero-dep, mock backend):**
- `glk_gestalt`: `8/9/10 == 1` when `sound_enabled`, `== 0` when not; `21 == 0`
  always.
- `glk_dispatch`: each `0x00F0–0x00FB` selector routes to the matching backend
  method with the right args (mock records calls); `0x00F8` forwards
  `(chan, snd, 1, 0)`, `0x00F9` forwards `(chan, snd, repeats, notify)`;
  `0x00FC` is a no-op returning 0; all return 0 when `sound_enabled` is false.
- `schannel_iterate` writes the rock to the out-ref pointer.
- `deliver_sound_notify`: when suspended at `glk_select`, writes
  `SOUND_NOTIFY{win:0, val1:snd, val2:notify}` into the event addr and clears
  `pending_input`; when not suspended, the event is queued and the next
  `glk_select` returns it.

**app — `AppGlk`:**
- `schannel_create` returns increasing nonzero refs; `get_rock` returns the
  stored rock; `iterate` walks channels in order and returns `(0,0)` past the
  end.
- `play/stop/set_volume/destroy` buffer the correct `SchannelOp`s in emission
  order; `take_sound_ops` drains and clears; `Play` carries the channel's
  current volume.

**app — `AppState`:**
- Glk-volume→gain is a pure function: `0 → 0.0`, `0x10000 → 1.0`,
  `0x8000 → 0.5`, `> 0x10000` passes through.
- op application updates `glulx_channels`/`glulx_sound_notify` correctly: a
  `Stop` after a `Play` clears the channel; replacing a sound on a busy channel
  drops the old notify entry. (Playback calls no-op without an audio device;
  assert the map bookkeeping.)

**audio crate:**
- gain combines with master; `set_sample_gain` adjusts a live sample's stored
  gain; repeat mapping (forever / N / once).

**Manual:** `sensory.blorb` "hit gong" plays the gong; the "does not support
sound" message no longer appears.

## Touched components (for planning)

- `crates/gvm/src/exec.rs` — `sound_enabled` + `set_sound`; gestalt arms
  `8/9/10`; schannel dispatch arms `0x00F0–0x00FC`; `deliver_sound_notify`.
- `crates/gvm/src/glk.rs` — new `GlkBackend` schannel methods (no-op defaults).
- `crates/app/src/glk_backend.rs` — `SchannelOp`; `AppGlk` channel state +
  method impls; `take_sound_ops`.
- `crates/app/src/glulx_session.rs` — `sound_enabled` param + `set_sound`;
  `finish_turn` drains sound ops; `sound_notify` method.
- `crates/app/src/session.rs` — `TurnResult.glulx_sound_ops`.
- `crates/app/src/state.rs` — `glulx_channels`, `glulx_sound_notify`,
  `play_glulx_sound_ops`, Glk-volume→gain helper.
- `crates/app/src/main.rs` — thread `sound_enabled` into 3 `GlulxSession::new`
  call sites; extend the `finished()` poll for Glulx notify; call
  `play_glulx_sound_ops` on the submit path.
- `crates/audio/src/lib.rs` — `play_sample_gain`, `set_sample_gain` (+ no-op
  stubs).
- `docs/superpowers/audits/2026-06-30-glulx-feature-gaps.md` — fix swapped
  selector ranges.

## Global constraints

- `zvm` and `gvm` crates stay **zero-dependency**; all audio decode/playback and
  the `SchannelOp` type live in the app crate. gvm's trait methods take only
  primitive args.
- Cross-platform (Windows/Linux/macOS); the `audio` crate already abstracts the
  backend; no platform-specific code added.
- README covers **major features only** — Glk/Glulx sound qualifies for a
  README note.
- No new UI element is introduced (playback reuses the existing sound
  volume/mute controls), so no new `style.toml` selector is required.
