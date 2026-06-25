# Sound & Diagnostic Event Visualization — Design

**Date:** 2026-06-25
**Status:** Approved, ready for planning

## Goal

Surface two classes of VM event that are currently invisible in the TUI:

1. **`sound_effect` beeps** — visualize the Z-machine `sound_effect` opcode
   (VAR:0x15) as a brief, themeable pulse of the **story-pane border**,
   distinguishing the high bleep (sound #1) from the low bleep (sound #2).
2. **Unimplemented-opcode warnings** — currently written to `eprintln!`
   (stderr, invisible under the TUI). Route them into the lower scrolling
   transcript, tagged as **meta** (not story).

Both share one mechanism: the VM records events into drainable queues, the
session surfaces them in `TurnResult`, and the app routes each to its
destination (border pulse for beeps, meta transcript line for warnings).

## Background

Today `sound_effect` falls through the VAR opcode dispatch to a catch-all that
warns once per opcode via `eprintln!` and ignores the call
(`crates/zvm/src/cpu/exec.rs`, the `warned_var_opcodes` fallthrough). Under the
ratatui TUI, stderr is not visible, so the player gets no feedback at all.

The app already has the infrastructure this design reuses:
- `pulse_border_color(elapsed)` in `crates/app/src/render/map.rs` drives the
  map border's red/green oscillation while a background-tidy job runs. The new
  sound pulse follows the same "compute a color override per frame" pattern but
  targets the **story** border and uses a one-shot fade rather than a sustained
  oscillation.
- `state.push_transcript_kind(text, TranscriptKind::Meta)` already appends
  meta-tagged transcript lines (the `▏` gutter, hidden by `/filter story`).

## Architecture

### 1. zvm — implement `sound_effect`, capture diagnostics

Replace the warn-and-ignore fallthrough for opcode 0x15 with a real handler.

Operands (ZMSD §9): `sound_effect number effect volume routine`.
- **number** selects the sound. Per ZMSD §9.4, **#1 = high-pitched bleep**,
  **#2 = low-pitched bleep**. Numbers ≥ 3 are sampled sounds that require a
  Blorb resource file (not yet supported).
- **effect / volume / routine** are not needed to render a bleep; they are
  parsed (to consume the operands) but otherwise unused for #1/#2.

Behavior:
- `number == 1` → record `Beep::High`.
- `number == 2` → record `Beep::Low`.
- `number >= 3` → record a diagnostic line, e.g.
  `"sampled sound #N not supported (needs Blorb)"` (warn-once per number is not
  required; a single generic dedup key is acceptable to avoid spam).
- `number == 0` and other edge values → no beep, no diagnostic (per spec, 0 is
  not a defined bleep).

New state on `Machine`:
```rust
pub enum Beep { High, Low }

// on Machine:
pub pending_beeps: Vec<Beep>,   // drained by the host each turn
pub diagnostics:   Vec<String>, // host-facing warning lines, drained each turn
```

The existing unimplemented-opcode fallthrough stops calling `eprintln!` and
instead pushes its message string into `diagnostics`, preserving the
`warned_var_opcodes` warn-once-per-opcode dedup so a repeated opcode does not
flood the queue.

**Consumers decide presentation; the engine never prints.**

### 2. zvm-cli — preserve current behavior

The headless CLI currently relied on the engine's `eprintln!` for warnings.
After this change the CLI drains `machine.diagnostics` and prints each line to
stderr, so its observable behavior is unchanged. (The CLI does not render
beeps; `pending_beeps` is simply ignored there.)

### 3. session — surface in `TurnResult`

After `run_until_input` returns, drain both VM queues into the turn result and
clear them on the VM:
```rust
// added to TurnResult:
pub beep: Option<Beep>,        // latest beep this turn (last wins)
pub diagnostics: Vec<String>,  // warning lines emitted this turn
```
`beep` takes the last element of `pending_beeps` (if a turn produced multiple
beeps, the most recent drives the pulse). `diagnostics` is the full drained
vector. Both `pending_beeps` and `diagnostics` are emptied after draining.

This applies to all turn entry points that call `run_until_input`
(`submit`, `submit_char`, and initial startup), so beeps/diagnostics emitted
during any of them are surfaced.

### 4. app — route the events

In the run loop, after each `TurnResult` (line and char paths both):
- For each line in `result.diagnostics`:
  `state.push_transcript_kind(line, TranscriptKind::Meta)`.
- If `result.beep` is `Some(kind)`:
  `state.sound_pulse = Some(SoundPulse { kind, started: Instant::now() })`.

New app state (`crates/app/src/state.rs`):
```rust
pub struct SoundPulse {
    pub kind: zvm::cpu::exec::Beep, // or a thin app-side mirror
    pub started: std::time::Instant,
}
// on AppState:
pub sound_pulse: Option<SoundPulse>,
```

### 5. Pulse rendering — story border, one-shot fade

New pure helper (placed beside `pulse_border_color`):
```rust
/// Fade from `beep_color` back to `normal` over `SOUND_PULSE_MS`.
/// Returns None once the pulse has elapsed (caller then clears it).
pub fn sound_pulse_color(
    beep_color: Color,
    normal: Color,
    elapsed: Duration,
) -> Option<Color>;
```
- `SOUND_PULSE_MS = 500`.
- At `elapsed == 0`: returns `beep_color` (full intensity).
- Linearly (or ease-out) lerps toward `normal` as `elapsed → SOUND_PULSE_MS`.
- At `elapsed >= SOUND_PULSE_MS`: returns `None`.

`normal` must be resolved to a concrete RGB to lerp toward; when the configured
border color is the terminal default (no RGB), fade toward the beep color's
dimmed value instead (implementation detail for the plan — the helper takes a
concrete `normal: Color` and the caller supplies a sensible fallback).

In the render section, the story-pane border color is overridden when a pulse
is active, at each of the three story `draw_pane_frame` sites
(`TranscriptFull`, `Split`, and any other story-border draw). If
`sound_pulse_color` returns `None`, the loop clears `state.sound_pulse` and the
border renders normally. This mirrors the existing `map_border_override`
pattern used for `tidy_job`.

The story border and the map border never contend: sound pulses the story
border, the tidy job pulses the map border.

### 6. Theming

Two new `ColorScheme` fields and matching style selectors:
- `sound_beep_high` — default **amber `(255, 180, 40)`**.
- `sound_beep_low` — default **cyan-blue `(60, 140, 220)`**.

Wired into `SELECTOR_FIELDS`, the style apply/export path, and
`DEFAULT_STYLE_TOML`, consistent with every other themeable color. Defaults are
chosen to be visually distinct from the red/green tidy pulse so the two signals
are never confused.

## Forward note: real audio via Blorb

This feature is the **visual** layer. When Blorb sound support lands (tracked in
TODO), the same `pending_beeps` / `sound_effect` plumbing must also drive
**actual audio playback**:
- Bleeps #1/#2 → play the real high/low bleep tones.
- Sampled sounds (number ≥ 3) → look up and play the `Snd ` resource from the
  Blorb file, honoring the `effect` (prepare/start/stop/finish), `volume`, and
  `routine` (finish callback) operands that this design currently parses but
  ignores.

The border pulse should **remain** as a complementary visual cue (useful with
audio muted or for accessibility). In short: this design adds the eyes; Blorb
sound later adds the ears, reusing the exact event channel built here.

## Testing

**zvm**
- `sound_effect` with number 1 records `Beep::High`; number 2 records
  `Beep::Low`; number ≥ 3 records a diagnostic and no beep; number 0 records
  neither.
- The unimplemented-opcode fallthrough pushes to `diagnostics` (not stderr) and
  still warns at most once per opcode.
- `pending_beeps` / `diagnostics` drain empty after the host reads them.
- **Repoint** the existing `unimplemented_var_opcode_is_warned_once` test: it
  currently uses 0x15 as its "unimplemented" example, which is now implemented —
  switch it to a genuinely unimplemented VAR opcode.

**session**
- `TurnResult.beep` reflects the last beep of the turn; `TurnResult.diagnostics`
  contains the drained lines; both VM queues are empty afterward.

**app**
- `sound_pulse_color` returns full beep color at elapsed 0, an intermediate
  color partway, and `None` at/after `SOUND_PULSE_MS`.
- A `TurnResult` with diagnostics pushes `Meta`-tagged transcript lines.
- A `TurnResult` with a beep sets `state.sound_pulse`; the story border override
  is applied while active and cleared when the pulse expires.

## Out of scope

- Actual audio playback (Blorb sound — separate TODO; see forward note).
- The `effect` / `volume` / `routine` operand semantics beyond parsing.
- Sound-bar / VU-meter visualization (rejected: `sound_effect` is a discrete
  event with no continuous level data to display).
- Pulsing the map border or whole outer frame for sound (story border only).
