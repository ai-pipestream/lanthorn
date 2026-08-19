# Audio Verify — Sound-Capability Advertisement + `/play-sound` Diagnostic

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Fix the likely root cause of "games never play sound" — `zvm` unconditionally clears the Z-machine sound-capability header bits, so well-behaved games never call `@sound_effect` — and add a `/play-sound` slash command to directly diagnose/exercise the audio pipeline (list Blorb `Snd` resources; play one with a step-by-step report) independent of whether a game ever calls the opcode.

**Architecture:** Mirror the existing `honor_game_colours` mechanism exactly: a plain `bool` on `zvm::Machine`, a setter that re-advertises immediately, and a free `screen.rs` function that flips the header bits. The app threads `sound_available = cfg.enable_sound` into `GameSession::new`. `zvm` stays zero-dependency; it never sees `blorb`/`audio` types, only a bool. `/play-sound` follows the `slash::COMMANDS` registry pattern (pure `dispatch` → `SlashOutcome::PlaySound(Option<u32>)`) handled in `main.rs::dispatch_slash_outcome`, printing transcript `Meta` lines like `/print-colors`.

## Approved decisions
- **Change 1 — host-gated on `cfg.enable_sound`** (NOT unconditional, NOT gated on blorb/backend presence). Faithful analog of `honor_game_colours`→`enable_sound`; zero startup reorder; advertising with no device is spec-harmless (opcode no-ops downstream), same as colour today. Since `enable_sound` defaults true, games will call `@sound_effect` again.
- **Change 2 — `/play-sound <n>` plays regardless of `enable_sound`** (it's an explicit diagnostic) but the report states the gate: `enable_sound: off (attempting playback anyway — diagnostic)`.

## Global Constraints
- `crates/zvm` stays ZERO external deps; never references `blorb::*`/`audio::*` — only a `bool`, exactly like `honor_game_colours`.
- No new Cargo deps for either change.
- Cross-platform (macOS/Windows/Linux); no platform-specific code.
- Commit trailers on EVERY commit:
  ```
  Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
- Validation (Change 1): confirm Flags1 bit 5 (v4+) + Flags2 bit 7 SET when `enable_sound=true` via zvm unit tests reading header 0x01/0x10.
- Validation (Change 2): `lanthorn <sound game>` → `/play-sound` lists resources; `/play-sound <n>` prints a step report ending `playback: started (sound id N)` (device) or `backend returned None` (headless).

## File Structure
| File | Responsibility |
|------|-----------------|
| `crates/zvm/src/screen.rs` | new `advertise_sound(mem,on)`; `init_header_caps` gains `sound_available: bool`; stop unconditionally clearing Flags1 bit5(v4+)/Flags2 bit7 |
| `crates/zvm/src/cpu/exec.rs` | `Machine.sound_available: bool`; `set_sound_available(bool)`; `init_caps()` forwards it |
| `crates/app/src/session.rs` | `GameSession::new` gains `sound_available: bool`; calls `set_sound_available` |
| `crates/app/src/main.rs` | 4 `GameSession::new` call sites pass `enable_sound`; new `SlashOutcome::PlaySound` arm |
| `crates/zvm-cli/src/main.rs` | `set_sound_available(sound_enabled)` after `build_machine` |
| `crates/app/src/slash.rs` | `SlashOutcome::PlaySound(Option<u32>)`; `play-sound` CommandSpec (Game); registry-count test |
| `crates/app/src/state.rs` | `sound_kind_to_format` → `pub(crate)`; `sound_kind_label`, `format_sound_resource_list`, `PlaySoundReport`, `format_play_sound_report` |

---

## Task 1: `zvm/screen.rs` — `advertise_sound` + `init_header_caps` sound param

`init_header_caps` at ~screen.rs:306-377 (called from exec.rs); `advertise_colour` at ~382-398. Update all ~14 test call sites of `init_header_caps` (lines 548,558,576,586,619,628,639,653,662,664,677,682,698,705) to insert a 3rd `bool` arg. Rename/trim `header_caps_flags2_clears_pictures_sound` (~646) to only assert the pictures bit.

**Interfaces produced:**
- `pub fn advertise_sound(mem: &mut Memory, on: bool)` — sets/clears Flags1 bit 5 (v4+ ONLY — v3 bit5 = "screen splitting", different capability) and Flags2 bit 7 (all versions).
- `pub fn init_header_caps(mem: &mut Memory, honor_game_colours: bool, sound_available: bool, interpreter_number: Option<u8>)`.

TDD: add `sound_bit_tracks_sound_available_flag_v5` (init with false → bits clear; true → bits set; advertise_sound(false) clears) and `sound_bit_v3_flags1_untouched_but_flags2_tracks` (v3 bit5 stays set; Flags2 bit7 tracks). RED: `cargo test -p zvm sound_bit_`. Implement: remove bit5 from the v4+ Flags1 clear-mask and bit7 from the Flags2 clear-mask; call `advertise_sound(mem, sound_available)` after `advertise_colour`; add `advertise_sound`:
```rust
pub fn advertise_sound(mem: &mut Memory, on: bool) {
    if mem.version() >= 4 {
        let f1 = mem.read_byte(0x01);
        let f1 = if on { f1 | (1 << 5) } else { f1 & !(1 << 5) };
        mem.write_byte(0x01, f1);
    }
    let f2 = mem.read_word(0x10);
    let f2 = if on { f2 | (1 << 7) } else { f2 & !(1 << 7) };
    mem.write_word(0x10, f2);
}
```
Commit: `feat(zvm): add advertise_sound, gate sound header bits on sound_available`

## Task 2: `zvm/cpu/exec.rs` — `Machine.sound_available` + `set_sound_available`

`Machine` fields ~96-149; struct literal in `with_output` ~176-197; `init_caps()` ~211-215; setter near `set_honor_game_colours` ~219-222; import at ~17.

**Interfaces:** `Machine.sound_available: bool` (default false); `pub fn set_sound_available(&mut self, on: bool)` (sets field + `advertise_sound(&mut self.mem, on)`). `init_caps()` forwards `self.sound_available` to `init_header_caps`. Add `advertise_sound` to the `use crate::screen::{...}` import.

TDD: `set_sound_available_advertises_and_clears`, `init_caps_forwards_sound_available_field` (reuse existing test-machine helper). Commit: `feat(zvm): Machine.sound_available + set_sound_available, mirrors honor_game_colours`

## Task 3: `app` — thread `cfg.enable_sound` into `GameSession::new`

`session.rs:166-172`. **Interface:** `GameSession::new(story, honor_game_colours, sound_available, interpreter_number)`; body calls `machine.set_sound_available(sound_available)` after `set_honor_game_colours`. Call sites: main.rs:1096 startup → `cfg.enable_sound`; main.rs:3427 reset_game → `state.config.enable_sound`; main.rs:4045 & 4080 hints-panel VM → `false` (hint browser never plays sound). TDD: `new_session_forwards_sound_available`. Commit: `feat(app): thread enable_sound into GameSession::new, advertise sound capability at startup`

## Task 4: `zvm-cli` parity

`zvm-cli/src/main.rs` after `set_honor_game_colours(honor)` (~766): add `machine.set_sound_available(sound_enabled);` (`sound_enabled = !args.no_sound` already at ~745). No new test (1-line parity); smoke-verify. Commit: `feat(zvm-cli): advertise sound capability, mirrors set_honor_game_colours`

## Task 5: `slash.rs` — `/play-sound` registration

`SlashOutcome` enum ~32-62; `COMMANDS` Game section ~134-170 (after `volume` ~170); `registry_is_complete_and_well_formed` test ~598-620 (Game count +1, total +1 — verify actual current counts).

**Interfaces:** `SlashOutcome::PlaySound(Option<u32>)` (None=list, Some(n)=play). CommandSpec `"play-sound"`, category `Game`, context `Global`, usage `"play-sound [n]"`, dispatch parses optional u32 → `PlaySound(None|Some(n))` or `err(...)` on non-numeric. TDD: `play_sound_command_parses_optional_number`, `play_sound_command_present`. Commit: `feat(app): register /play-sound diagnostic command`

## Task 6: `state.rs` — pure formatting helpers

After `sound_kind_to_format` (~239-246); bump it to `pub(crate)`.

**Interfaces produced (all pure, unit-testable without a device):**
- `pub fn sound_kind_label(k: blorb::SoundKind) -> &'static str` (Aiff→"AIFF", Ogg→"OGG", Mod→"MOD", Other→"other").
- `pub fn format_sound_resource_list(blorb: Option<&blorb::Blorb>) -> Vec<String>` — None→"no sound blorb resolved"; filter `resources()` by `usage == b"Snd "`; empty→"no Snd resources"; else header `"{n} sound resource(s):"` + per-resource `"  #{num} {label} {len} bytes  {playable|not decodable}"` (playable = `sound_kind_to_format(kind).is_some()`). Map chunk_type FORM→Aiff, OGGV→Ogg, "MOD "→Mod, else Other.
- `pub struct PlaySoundReport { number:u32, enable_sound:bool, backend_present:bool, blorb_present:bool, resource:Option<(blorb::SoundKind,usize)>, format:Option<audio::SoundFormat>, sound_id:Option<audio::SoundId> }` (Debug, Default, Clone).
- `pub fn format_play_sound_report(r: &PlaySoundReport) -> Vec<String>` — lines: header; `enable_sound: on|off (attempting playback anyway — diagnostic)`; `audio backend: present|NONE`; `sound blorb: resolved|NONE`; then resource NOT FOUND (early return) | `found, kind=…, N bytes`; then format not decodable (early return) | `format: {:?} — decodable`; then `playback: started (sound id N)` | `backend returned None`.

TDD (`mod play_sound_tests`): build a fixture blorb (mirror `blorb::tests` IFF/RIdx builder); assert list marks #3 AIFF playable, #5 OGG playable, #9 other not-decodable; report variants: not-found, undecodable kind, success shows "sound id 7", disabled gate shows "off (attempting playback anyway". Commit: `feat(app): pure formatters for the /play-sound diagnostic`

## Task 7: `main.rs` — wire `SlashOutcome::PlaySound`

`dispatch_slash_outcome` (~3164-3411), add arm after `PrintColors` (~3195). None → print `format_sound_resource_list(state.sound_blorb.as_ref())` as `TranscriptKind::Meta` lines. Some(n) → build `PlaySoundReport{number:n, enable_sound:state.config.enable_sound, backend_present:state.audio.is_some(), blorb_present:state.sound_blorb.is_some(), ..}`; if `state.sound_blorb` has `sound(n)` → set resource; if `sound_kind_to_format(kind)` Some → set format; if `state.audio.as_mut()` → `play_sample(bytes, fmt, 8, 1)` → set sound_id; print `format_play_sound_report(&report)` as Meta lines. (Disjoint-field borrows of `&mut AppState` like `play_turn_sounds`.) z_volume=8 (loudest explicit scale, matches SoundEvent default). Impure glue — not unit-testable; build clean + manual verify. Commit: `feat(app): wire /play-sound to the audio backend + sound blorb`

## Self-Review notes
- Type consistency verified: `init_header_caps(mem, honor:bool, sound:bool, interp:Option<u8>)` T1→T2; `Machine.sound_available`/`set_sound_available` T2→T3-4; `GameSession::new(story,honor,sound,interp)` T3→all call sites; `SlashOutcome::PlaySound(Option<u32>)` T5→T7; `PlaySoundReport` fields T6→T7.
- Implementers must locate & reuse existing test fixtures/helpers (sample story bytes, test-machine constructor, blorb builder) rather than inventing them; verify actual line numbers/counts before editing.
