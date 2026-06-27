# Animation Engine (Foundation) + Smooth Scroll — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Sequencing:** runs AFTER the keyboard wave (needs its `TranscriptScrollPage`)
and the mouse-wheel wave (shared `main.rs`/`state.rs`). Implement once both merge.

## Goal

A small, reusable animation engine for the TUI, plus one validating consumer
(smooth transcript scrolling). The engine gives later effects (dialog open/close,
layout transitions, dock slide-ins) a shared timing/easing primitive and run-loop
integration. This spec covers ONLY the engine + smooth scroll; other effects are
separate follow-on specs.

## Background (current code)

- Time-based effects exist but are bespoke per-effect: the sound-beep border flash
  (`SoundPulse { started: Instant }` + `sound_pulse_color`, render/map.rs:84) and
  the tidy border pulse (`pulse_border_color`, render/map.rs:59). Each has its own
  `started: Instant`, color function, and an entry in the run-loop's fast-poll
  predicate: `poll_ms = if tidy_job.is_some() || sound_pulse.is_some()
  { TIDY_POLL_MS } else { 50 }` (main.rs:1291). While fast-polling, the loop
  redraws each iteration, so time-based effects advance without input.
- There is no shared easing helper and no `[animation]` config.
- Transcript scrolling is instant: `state.transcript_scroll` (clamped to
  `[0, max_scroll]` after each draw). Scroll requests: mouse wheel
  `TranscriptScroll(±1)` and (from the keyboard wave) `TranscriptScrollPage(±1)`.
- TUI reality: animation is line-quantized; no sub-cell/pixel scrolling.

## Design

### 1. `anim` module (new: `crates/app/src/anim.rs`)

Pure, testable timing primitives — no rendering, no app state.

```rust
pub enum Easing { Linear, EaseIn, EaseOut, EaseInOut }

/// Map progress t in [0,1] through the easing curve; returns [0,1].
pub fn ease(curve: Easing, t: f64) -> f64;

/// Parse a config token ("linear" | "ease-in" | "ease-out" | "ease-in-out").
/// Unknown -> EaseOut (the default).
pub fn parse_easing(s: &str) -> Easing;

/// Linear interpolation.
pub fn lerp(from: f64, to: f64, t: f64) -> f64;  // from + (to-from)*t

pub struct Tween { started: Instant, duration: Duration, easing: Easing }
impl Tween {
    pub fn new(duration: Duration, easing: Easing) -> Self;  // started = Instant::now()
    pub fn progress(&self) -> f64;  // ease(easing, clamp01(elapsed/duration))
    pub fn done(&self) -> bool;      // elapsed >= duration
}
```

`ease`/`lerp`/`parse_easing` are the unit-tested core. `Tween` timing is thin
(constructed with a short duration for tests).

### 2. `[animation]` config (config.toml)

A flat sub-struct on `Config`, parsed from an `[animation]` TOML table:

```rust
pub struct AnimationConfig {
    pub enabled: bool,   // default true; false = every animation is instant
    pub easing: Easing,  // default EaseOut (parsed via parse_easing)
    pub scroll_ms: u64,  // default 120; smooth-scroll duration
}
```

```toml
[animation]
enabled = true
easing = "ease-out"
scroll_ms = 120
```

`Config` gains `pub animation: AnimationConfig` with the above defaults, included
in the file-merge and the format-preserving `write_config`. Easing serializes via
its token string.

### 3. Run-loop integration

Replace the hard-coded fast-poll predicate with a single helper:

```rust
// AppState
pub fn has_active_animation(&self) -> bool {
    self.tidy_job.is_some() || self.sound_pulse.is_some() || self.scroll_anim.is_some()
}
```

`poll_ms = if state.has_active_animation() { ANIM_POLL_MS } else { 50 }`
(`ANIM_POLL_MS` = the existing `TIDY_POLL_MS`, ~30fps). The loop already redraws
each iteration; after a poll timeout it advances animations (clears any that are
`done()`). The existing `sound_pulse`/`tidy_job` effects are NOT rewritten — they
are simply included in `has_active_animation()`.

### 4. Smooth scroll (validating consumer)

The transcript scroll offset eases to its target instead of jumping.

- `state.transcript_scroll` remains the logical TARGET offset (set by the scroll
  actions as today). A new `state.scroll_anim: Option<ScrollAnim>` holds the
  in-flight animation: `ScrollAnim { from: f64, to: f64, tween: Tween }`.
- `ScrollAnim::current() -> f64` = `lerp(from, to, tween.progress())`.
- When a scroll action changes the target AND `config.animation.enabled` AND
  `scroll_ms > 0`: arm/retarget `scroll_anim` with `from = current displayed
  offset` (the live `scroll_anim.current()` if animating, else `transcript_scroll`),
  `to = new clamped target`, a fresh `Tween(scroll_ms, easing)`. Otherwise (disabled
  or zero), apply instantly (clear `scroll_anim`).
- Rendering: the transcript uses the **effective** offset
  `scroll_anim.map(|a| a.current().round() as u16).unwrap_or(transcript_scroll)` —
  line-rounded. The existing `[0, max_scroll]` clamp still applies.
- The run loop, while `scroll_anim` is `Some`, advances it each frame; when
  `tween.done()`, set `transcript_scroll = to` and clear `scroll_anim`.
- Only the transcript animates. Modal-list wheel moves discrete selection (no
  offset), so it stays instant. Map pan/zoom is unchanged.

## Testing

- `ease`: each curve maps `t=0 -> 0`, `t=1 -> 1`; `EaseIn`/`EaseOut` are below/above
  the diagonal at `t=0.5` (assert the inequality, non-vacuous); `Linear` is identity.
- `lerp` and `parse_easing` (incl. unknown -> EaseOut).
- `Tween`: `progress()` is 0 at start and reaches ~1 after the duration (short
  duration + a brief sleep, or assert `done()` flips); clamped past the end.
- `AnimationConfig`: parses an `[animation]` table; defaults applied when absent;
  `write_config` round-trips `enabled`/`easing`/`scroll_ms`.
- `has_active_animation`: true when any of the three sources is set, false otherwise.
- Smooth scroll: a scroll request with animation enabled arms `scroll_anim`
  (from=old, to=clamped target); `current()` interpolates; retarget mid-flight
  starts from the current displayed offset; with `enabled=false` (or `scroll_ms=0`)
  the offset jumps and `scroll_anim` stays `None`; the effective offset is
  line-rounded and clamped to `[0, max_scroll]`.

## Out of scope

- Migrating the existing sound/tidy pulses onto `Tween` (future cleanup).
- Dialog open/close, layout transitions, dock slide-ins — separate follow-on specs
  that consume this engine.
- Animating modal-list selection, map pan/zoom, or anything sub-cell.

## Global constraints

- 0 warnings + full `cargo test -p app` green per task.
- Commit-only on local `main`; TDD wave. No push without explicit instruction.
- Commit trailers, every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`;
  no backticks in commit bodies.
- Surgical changes; do not edit `TODO.md` during the wave.
- Default behavior with `[animation] enabled = true` must keep scrolling usable
  (short, snappy); `enabled = false` must exactly reproduce today's instant scroll.
